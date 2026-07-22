# Code Pipeline — design

Spec-driven, model-tiered coding as the **default coding path** in
orchestratectl. Expensive thinking is done once (spec) and amortized across
cheap implementation (code); a heavy model verifies; findings loop back until
the product matches the *original intent*.

Status: DESIGN (iterating). Implementation follows once locked.

---

## 0. Design principles (project-wide, surfaced while shaping this)

These govern this pipeline and are meant to generalize to the rest of
orchestratectl.

1. **Trust model judgment over brittle rules.** Where a decision needs an
   estimate ("is this chunk too big?", "does this warrant another review
   round?"), instruct the agent to use common sense — *not* a hardcoded
   numeric threshold. We ask the model to be sensible and rely on its
   intuition. Do NOT write rules like ">2 files or a separate interface → split";
   write "split into appropriately-sized pieces by your own judgment of what a
   sensible software task is."
2. **Concentrate decisions in the expensive model.** Every point where a
   judgment is made belongs to a capable model (Opus). Cheap models only do
   mechanical work that contains no decisions.
3. **Single human-interaction locus.** All conversation with the human flows
   through *one* front-end agent (the one running `/stint` etc.). No other agent
   ever talks to the human directly — it escalates *up* the chain, and the
   front-end decides what actually reaches the person. There is no "dumb" human
   gate (e.g. a human typing a merge command when everything is already resolved).
4. **Self-improving tooling.** When an agent hits a limitation in a contract it
   depends on (the `plan.json` schema, a CLI surface, a report shape), it files a
   feedback issue into the **orchestratectl repo** describing the gap, rather than
   working around it silently. The software improves itself on the fly through the
   work it runs.

---

## 1. The invariant: intent, not spec

The anchor is the **original intent** of the feature — what the user actually
wants to exist. The **spec is only a draft** of that intent: it gains
corrections and refinements along the way, most surfaced during verification.
When code reveals the spec is wrong, we update the spec — but the thing held
constant is the intent. Convergence is "product matches intent," never "spec was
implemented verbatim" and never "verify came back clean."

**Verify is almost never "ok"** — an Opus verify always produces material, so the
verify→triage→fix loop is the *normal* cycle, not an exception. But the loop is
kept short by judgment, not by a counter:

- After each fix, the **orchestrator heuristically decides** whether the change
  warrants another verify round, by estimating the scope of what changed
  (common-sense assessment — no fixed iteration number). A tiny, contained fix may
  need no re-verify; a broad change does.
- **Two fix rounds is already a lot.** If the product isn't converging by then,
  that is a signal to escalate (§7), not to keep grinding.

Convergence = *no remaining must-fix findings* (the rest triaged to
spin-offs / drops), as judged by the orchestrator — not an empty verify.

---

## 2. Model tiers (first cut)

| Role | Model (v1) | Makes decisions? | Why |
|---|---|---|---|
| **feature-orchestrator** | **Opus** | **yes — all of them** | holds the intent, triages findings, resolves/escalates, decides "matches intent" |
| **spec** | **Opus** | yes (architecture) | codebase understanding + design + chunking; done once, amortized |
| **code** | **deepseek-flash** | **no** | cheap/fast bulk implementation from a self-contained brief; other models tested later |
| **verify** | **Opus** | yes (what's must-fix) | catches what a cheap coder misses; produces the findings that drive the loop |

- The code tier is a **pluggable binding**, not hardcoded. v1 = deepseek-flash;
  the tier→agent-command resolution is config so we can A/B other models without
  touching the pipeline.
- **The code node makes no decisions and does not self-merge.** It writes code
  from its brief, runs a local self-check (build/lint), and commits its chunk
  branch. The **supervisor** merges the chunk into the integration branch
  mechanically; the code node never runs `run merge` and never triages anything.
  This keeps every decision in an Opus context (principle 2) and shrinks what the
  cheap model must be capable of.
- **Adaptive promotion:** a chunk that fails verify twice is re-run at a higher
  tier (deepseek-flash → mid → Opus). Self-healing without a human.

### Infra prerequisite (foundational — build first)

`run create` carries no model info today. The hook exists: `create.sh --agent
<agent-cmd>` → `workmux add -a`. So per-node model selection is a thin addition:

```
run create --model <tier|agent-cmd>  →  create.sh --agent "<launcher for that model>"
```

deepseek-flash is **not** a Claude model, so the agent-command is not `claude
--model X` — it is whatever launcher speaks to that model. Because the code node
is now *pure implementation* (write + commit, no self-merge, no report), the bar
for the cheap launcher is low: **can it read a brief, edit files, and commit in a
worktree?** — no need to honor the full node.report / run-merge contract. That is
the (reduced) subject of the task-0 spike.

---

## 3. Contexts and who spawns whom

```
USER  ──talks only to──►  FRONT-END AGENT  (the human's conversation; /stint etc.)
                             │  decides WHICH features to build; never sees code
                             │  the SOLE point of human interaction (principle 3)
                             │
                             │  spawns one FEATURE-ORCHESTRATOR per feature
                             ▼
                        FEATURE-ORCHESTRATOR  [Opus]  (one feature, holds the INTENT)
                             │  the decision brain: kicks off the pipeline, triages
                             │  verify findings, routes FIX/DISCUSS/SPIN_OFF, updates
                             │  the spec, decides "matches intent"; escalates UP to the
                             │  front-end when a human call is genuinely needed.
                             │  stays LEAN — reads compact reports + findings, never diffs
                             │
                             │  drives the mechanical sequence via ↓
                             ▼
                        SUPERVISOR STATE MACHINE  (no LLM; muscle)
                             │  spawns stages per plan.json; MERGES chunk branches;
                             │  does teardown
                             ├─ spec-node   [Opus]            → plan.json + spec.md
                             ├─ code-node×N [deepseek-flash]  → commit chunk branch (no merge)
                             └─ verify-node [Opus]            → findings (node.report assets)
```

The **inner sequence** (spec→code→verify, merges, teardown) is the supervisor
state machine — zero coordination tokens. The **outer adaptive loop** (findings →
action, spec updates, intent check, escalation) is the Opus feature-orchestrator.
Muscle vs. brain.

### Front-end vs orchestrator

- **Default:** `/stint` (front-end) **spawns** a separate feature-orchestrator per
  feature. Keeps /stint's context clean (only per-feature "done"), and pins each
  orchestrator to one intent + one integration branch.
- **Alternative:** `/stint` *is* the orchestrator (simpler, one-off single-feature
  use). Not the default — it dilutes the intent-anchor and re-pollutes context.

### One orchestrator = one feature

A feature-orchestrator is scoped to **one** feature (one intent, one integration
branch, one findings-loop). Multi-feature concurrency = the front-end spawns
**several** single-feature orchestrators in parallel (existing parent→child
fan-out). A multiplexing orchestrator would blur intents; rejected.

---

## 4. Call diagram (refined sequence)

```
 user ──"build feature X (intent)"──► FRONT-END (/stint)   ◄── only human touchpoint
                                          │
                                          │ run create (feature-orchestrator, Opus)
                                          │   ↳ forks INTEGRATION BRANCH  feat/<slug>  off source (main)
                                          ▼
                                    FEATURE-ORCHESTRATOR [Opus]  (holds intent)
                                          │
             ┌────────────────────────────┤  (drives supervisor state machine)
             │                             │
   VAIHE 1   │  spawn spec-node [Opus] ────────────────► reads codebase on feat/<slug>
             │                             │              writes plan.json (chunk DAG, per-chunk
             │                             │              self-contained briefs, tier hints,
             │                             │              verify criteria) + spec.md.
             │                             │              Chunks sized by MODEL JUDGMENT — no rules.
             │                             ◄── node.report "spec ready, N chunks"
             │                             │
             │   ▓ optional: if arch is significant/uncertain, orchestrator escalates
             │     UP to front-end → human weighs in → else auto-proceed ▓
             │                             │
   VAIHE 2   │  supervisor spawns code-nodes per plan.json DAG [deepseek-flash]
             │     ├─ chunk-1 (indep) ─┐ parallel  → self-check + commit chunk branch
             │     ├─ chunk-2 (indep) ─┘           → SUPERVISOR merges → feat/<slug>
             │     └─ chunk-3 (dep on 1) ─ sequential, rebased on updated feat/<slug>
             │        (code node makes no decisions, never self-merges)
             │                             │
   VAIHE 3   │  spawn verify-node [Opus] on feat/<slug> tip
             │     runs tests+clippy, checks product-vs-INTENT (not just vs spec)
             │     emits findings as node.report assets:
             │        fix_items[]  discussion_items[]  spinoff_proposals[]  (+ drops)
             │                             ◄── findings
             │                             │
   ┌─────────┤  ORCHESTRATOR [Opus] TRIAGES findings  (/assess-findings-style)  ◄── THE LOOP
   │ FIX / FIX_WITH_CARE ─► re-spawn the offending code chunk with findings in its brief
   │                        (≤~2 rounds; promote tier on repeat fail)
   │ SPEC-FLAW           ─► re-spawn spec-node to UPDATE plan.json/spec.md against INTENT,
   │                        then re-code affected chunks  (spec = living draft)
   │ DISCUSS             ─► orchestrator resolves autonomously if it can; else escalates
   │                        UP to front-end → human (judgment-level only)
   │ SPIN_OFF            ─► record proposal (non-blocking; deferred backlog, not this feature)
   │ DROP                ─► recorded, no action
   │  ↳ orchestrator decides by change-scope whether a re-verify is even needed
   └─► loop until NO must-fix findings remain AND product matches intent
             │        (>2 non-converging rounds ⇒ escalate UP, don't grind)
             │                             │
   VAIHE 4   │  merge feat/<slug> → source (AUTOMATIC — no human gate) ; integration branch dies
             ▼
      node.report (rollup: what shipped, deferred spin-offs, any open discussions)
                                          │
                                          ▼
                              FRONT-END sees "X done + tiivistelmä"
```

---

## 5. Integration-branch lifecycle

| Phase | Branch event |
|---|---|
| Feature-orchestrator run created | fork `feat/<slug>` off `source_branch` (recorded in manifest) |
| Spec | spec-node reads on `feat/<slug>`; commits `plan.json` + `spec.md` there |
| Code | each chunk forks `wt/<id>-chunk-k` off current `feat/<slug>`; code node commits; **supervisor merges** back; sequential chunks rebase on the moved tip |
| Verify | runs on `feat/<slug>` tip (read + test) |
| Fix loop | re-code commits land on `feat/<slug>`; spec updates re-commit `plan.json` |
| Converged | **automatic** merge `feat/<slug>` → `source_branch` (no human gate) |
| Teardown | supervisor removes `feat/<slug>` + chunk worktrees/branches/tmux |

Born at run creation, lives through the whole spec/code/verify/fix loop, dies at
final merge — mirroring `/orchestrate`'s integration branch (the machinery reused).

---

## 6. Verify → findings → action (the /assess-findings mapping)

Verify output is triaged exactly like a review-findings list; the orchestrator IS
the triage step and the on-disk primitives already model each disposition.

| /assess verdict | orchestrator action | orchestratectl primitive | blocks feature? |
|---|---|---|---|
| FIX (clean) | re-code the chunk, findings in brief | new code-node iteration | yes (must converge) |
| FIX_WITH_CARE | re-code, tier promoted / narrower brief | code-node (higher tier) | yes |
| SPEC-FLAW | re-spec against intent, then re-code | spec-node iteration → plan.json update | yes |
| DISCUSS | resolve autonomously, else escalate UP to front-end | `Discussion` (open→resolved) | maybe |
| SPIN_OFF | record for later; do not block | `SpinoffProposal` (proposed→approved/rejected) | no |
| DROP | note, no action | wrap_up_recommendations | no |

Because verify is never empty, the orchestrator's job is to *sort the inevitable
findings* into must-fix-now (loop) / defer (spin-off) / not-worth-it (drop) /
needs-a-human (discuss) — /assess-findings run continuously inside the build loop —
and to judge, by change scope, whether another verify round is even warranted.

---

## 7. Human interaction (single locus, no dumb gates)

**The human talks only to the front-end agent.** Everything else escalates *up*.

- Sub-agents (spec, verify) and the feature-orchestrator **never address the human
  directly.** A finding or question the orchestrator cannot resolve becomes a
  `Discussion` that bubbles **up to the front-end**, which decides whether it truly
  needs the person. Escalation chain: `verify → orchestrator → front-end → human`.
- **No human-gated final merge.** By the time the feature converges, everything is
  already resolved — asking a human to type a merge command is pointless ceremony.
  Vaihe 4 merges automatically. The `code` kind's old "human runs /worktree-merge"
  step is removed for the pipeline.
- The **only** things that reach the human are genuine judgment calls the
  front-end chooses to forward:
  1. **Feature request + intent** (entry).
  2. **Architecture question** — the orchestrator is unsure about a significant
     design choice after spec; escalates up. Optional, often skipped.
  3. **A specific mid-flight decision** a sub-task genuinely can't make — routed up
     to the front-end, surfaced only if it needs the person.
  4. **Intent unreachable** — non-convergence / budget exhausted; the orchestrator
     stops and escalates up rather than grinding.
- Spin-offs are async backlog reviewed later via `/assess-findings` / `/stint` —
  never a build-time gate.

Coding itself is **never** monitored by a human; every human touch is at the
design/judgment level and arrives through the one front-end conversation.

---

## 8. Scope: direct into the existing pipeline

Build this **directly into the existing coding kinds**, not a walled-off greenfield
kind.

1. Land the model-tier plumbing (task 0) + the staged supervisor (incl.
   supervisor-side chunk merge) + spec/code/verify roles.
2. Make the **coding** kinds (`code`, `spinoff`, `bugfix`) run through the
   spec→code→verify pipeline as their default execution model.
3. Non-coding kinds (research, technical-decision, make-skill, fan-out) unaffected.

---

## 9. `plan.json` — schema now, extensible + self-improving

Design `plan.json` for **current** needs, but versioned and forward-compatible:

- `schema_version` on the file. Readers tolerate unknown fields (forward-compat)
  and branch on the version.
- Draft fields: `chunk id · deps · self-contained brief · tier · verify criteria ·
  files-touched (hint)`. Kept minimal; grown as real needs appear.
- **Not dynamically extensible at runtime** — and that's fine. Instead, when a
  spec/verify agent finds the schema **insufficient** for something it needs to
  express, it **files a feedback issue into the orchestratectl repo** (via
  `issuectl`, type improvement, describing the missing field/shape). The schema
  then grows deliberately in a later version. This is principle 4 in action: the
  pipeline improves its own contracts through the work it runs.

---

## 10. Open questions (remaining)

- **deepseek-flash launcher (task-0 spike, reduced scope):** what agent-command
  makes a non-Claude model read a brief, edit files, and commit in a worktree?
  (No longer needs the full self-merge/report contract — supervisor merges.)
  Biggest remaining unknown; do this spike first.
- **`plan.json` concrete draft:** lock the v1 field list + versioning convention
  before implementation.
- **Supervisor-side chunk merge:** new capability (supervisor performs the
  integration merge instead of the node). Scope the failure/rebase handling.
- **Spec-flaw vs FIX boundary:** how verify signals "the spec itself is wrong"
  (re-spec) vs "the code is wrong" (re-code) — a field on the finding, or the
  orchestrator's judgment. Lean on orchestrator judgment (principle 1).

Resolved this round: convergence is orchestrator-judged by change scope (≤~2
rounds, no counter); chunking is model judgment (no numeric rules); orchestrator =
Opus (all decisions), code = deepseek-flash pure-impl (no decisions/merge);
final merge automatic (no human gate); all human interaction via the single
front-end locus; schema extensibility via self-filed feedback issues.
