# Code Pipeline — design

Spec-driven, model-tiered coding as the **default coding path** in
orchestratectl. Expensive thinking is done once (spec) and amortized across
cheap implementation (code); a heavy model verifies; findings loop back until
the product matches the *original intent*.

Status: DESIGN (iterating). Implementation follows once locked.

---

## 1. The invariant: intent, not spec

The anchor is the **original intent** of the feature — what the user actually
wants to exist. The **spec is only a draft** of that intent: it will gain
corrections and refinements along the way, most of them surfaced during
verification. When code reveals the spec is wrong, we update the spec — but the
thing we hold constant is the intent. Convergence is "product matches intent,"
never "spec was implemented verbatim" and never "verify came back clean."

Corollary (from the owner): **verify is almost never "ok"** — an Opus verify
always produces material. So the verify→triage→fix loop is the *normal* cycle,
not an exception. Convergence = *no remaining must-fix findings*, with the rest
triaged to spin-offs / drops — not an empty verify.

---

## 2. Model tiers (first cut)

| Stage  | Model (v1)      | Why |
|--------|-----------------|-----|
| spec   | **Opus**        | codebase understanding + architecture; done once, amortized |
| code   | **deepseek-flash** | cheap/fast bulk implementation from a self-contained brief; other models tested later |
| verify | **Opus**        | catches what a cheap coder misses; produces the findings that drive the loop |

- The code tier is a **pluggable binding**, not hardcoded. v1 = deepseek-flash;
  the tier→agent-command resolution is a config surface so we can A/B other
  models without touching the pipeline.
- **Adaptive promotion:** a chunk that fails verify twice is re-run at a higher
  tier (deepseek-flash → mid → Opus). Self-healing without a human.

### Infra prerequisite (foundational — build first)

`run create` carries no model info today. The hook exists: `create.sh --agent
<agent-cmd>` → `workmux add -a`. So per-node model selection is a thin addition:

```
run create --model <tier|agent-cmd>  →  create.sh --agent "<launcher for that model>"
```

Note: deepseek-flash is **not** a Claude model, so the agent-command is not
`claude --model X` — it is whatever launcher speaks to that model (a different
CLI, or Claude Code pointed through a model router/proxy). The design treats
"code tier" abstractly as *a resolved agent-command*; the binding lives in
config. **This plumbing is task 0 — nothing else works without it.**

---

## 3. Contexts and who spawns whom

Three layers. The middle layer is the new thing.

```
USER FRONT-END AGENT  (the conversation the human is in; runs /stint etc.)
   │  decides WHICH features to build; never sees code
   │
   │  spawns one FEATURE-ORCHESTRATOR per feature  (parallel features = parallel orchestrators)
   ▼
FEATURE-ORCHESTRATOR  (its own agent context, ONE feature, holds the INTENT)
   │  the adaptive brain: kicks off the pipeline, triages verify findings,
   │  routes FIX/DISCUSS/SPIN_OFF, updates the spec, decides "matches intent"
   │  stays LEAN — reads compact reports + findings, never diffs
   │
   │  drives the mechanical sequence via ↓
   ▼
SUPERVISOR STATE MACHINE  (no LLM; muscle)
   │  spawns stages per plan.json, does merges + teardown
   ├─ spec-node   [Opus]           → plan.json + spec.md
   ├─ code-node×N [deepseek-flash] → chunk branches → integration branch
   └─ verify-node [Opus]           → findings (as node.report assets)
```

Two-layer split honors both of the owner's asks: the **inner sequence** (spec→
code→verify, merges, teardown) is a supervisor state machine — zero coordination
tokens; the **outer adaptive loop** (findings → action, spec updates, intent
check) is the feature-orchestrator agent. The orchestrator is "the brain," the
supervisor is "the plumbing."

### Front-end vs orchestrator (the two options)

- **Recommended:** `/stint` (front-end) **spawns** a separate feature-orchestrator
  per feature. Keeps /stint's context clean (it sees only per-feature "done"),
  and each orchestrator's context is pinned to one intent + one integration
  branch → sharp intent-anchoring.
- **Alternative:** `/stint` *is* the orchestrator. Simpler, but /stint's context
  then carries every feature's findings-loop → dilutes the intent-anchor and the
  context-hygiene win. Rejected as the default; available for one-off single-feature use.

### Can one orchestrator handle many features at once?

No — by design a feature-orchestrator is scoped to **one** feature (one intent,
one integration branch, one findings-loop). Multi-feature concurrency = the
front-end spawns **several** single-feature orchestrators in parallel (the
existing parent→child fan-out model). Rationale: a multiplexing orchestrator
would blur intents and re-pollute the context we are trying to keep clean.
Trade-off: N orchestrator contexts cost more than one; acceptable because each is
lean (reports, not diffs) and the isolation is the whole point.

---

## 4. Call diagram (refined sequence)

```
 user ──"build feature X (intent)"──► FRONT-END (/stint)
                                          │
                                          │ run create --kind <coding> (feature-orchestrator)
                                          │   ↳ forks INTEGRATION BRANCH  feat/<slug>  off source (main)
                                          ▼
                                    FEATURE-ORCHESTRATOR  (holds intent)
                                          │
             ┌────────────────────────────┤  (drives supervisor state machine)
             │                             │
   VAIHE 1   │  spawn spec-node [Opus] ────────────────► reads codebase on feat/<slug>
             │                             │              writes plan.json (chunk DAG, per-chunk
             │                             │              self-contained briefs, tier hints,
             │                             │              verify criteria) + spec.md
             │                             ◄── node.report "spec ready, N chunks"
             │                             │
             │   ▓ USER-INTERACTION POINT #1 (OPTIONAL): architecture audit ▓
             │   if arch is significant/uncertain → open discussion(severity=critical)
             │   → human resolves → else auto-proceed. (The ONLY routine human touch.)
             │                             │
   VAIHE 2   │  supervisor spawns code-nodes per plan.json DAG [deepseek-flash]
             │     ├─ chunk-1 (indep) ─┐ parallel  ─► merge → feat/<slug>
             │     ├─ chunk-2 (indep) ─┘           ─► merge → feat/<slug>
             │     └─ chunk-3 (dep on 1) ─ sequential, rebased on updated feat/<slug>
             │        each: self-check (build/lint) then commit; node.report "chunk done"
             │                             │
   VAIHE 3   │  spawn verify-node [Opus] on feat/<slug> tip
             │     runs tests+clippy, checks product-vs-INTENT (not just vs spec)
             │     emits findings as node.report assets:
             │        fix_items[]  discussion_items[]  spinoff_proposals[]  (+ drops)
             │                             ◄── findings
             │                             │
   ┌─────────┤  ORCHESTRATOR TRIAGES findings  (/assess-findings-style)  ◄── THE LOOP
   │ FIX / FIX_WITH_CARE ─► re-spawn the offending code chunk with findings appended
   │                        to its brief (bounded retries; promote tier on repeat fail)
   │ SPEC-FLAW           ─► re-spawn spec-node to UPDATE plan.json/spec.md against INTENT,
   │                        then re-code affected chunks  (spec = living draft)
   │ DISCUSS             ─► open discussion → orchestrator resolves autonomously if it can,
   │                        else ▓ USER-INTERACTION POINT #2 ▓ (human, judgment-level only)
   │ SPIN_OFF            ─► open spinoff proposal (non-blocking; deferred backlog, not this feature)
   │ DROP                ─► recorded, no action
   └─► re-verify ──► loop until NO must-fix findings remain AND product matches intent
             │        (bounded by iteration/token budget; ▓ POINT #3 ▓ escalate if intent unreachable)
             │                             │
   VAIHE 4   │  merge feat/<slug> → source (existing run merge) ; integration branch dies
             ▼
      node.report (rollup: what shipped, deferred spin-offs, any open discussions)
                                          │
                                          ▼
                              FRONT-END sees "X done + tiivistelmä"
                              (coding was NEVER human-monitored; only design/judgment was)
```

---

## 5. Integration-branch lifecycle (was unspecified)

| Phase | Branch event |
|---|---|
| Feature-orchestrator run created | fork `feat/<slug>` off `source_branch` (recorded in manifest) |
| Spec | spec-node reads on `feat/<slug>`; commits `plan.json` + `spec.md` there (or run dir) |
| Code | each chunk forks `wt/<id>-chunk-k` off current `feat/<slug>`; merges back; sequential chunks rebase on the moved tip |
| Verify | runs on `feat/<slug>` tip (read + test) |
| Fix loop | re-code commits land on `feat/<slug>`; spec updates re-commit `plan.json` |
| Converged | merge `feat/<slug>` → `source_branch` (run merge machinery) |
| Teardown | supervisor removes `feat/<slug>` + chunk worktrees/branches/tmux |

The integration branch is **born at run creation, lives through the entire
spec/code/verify/fix loop, dies at final merge** — mirroring `/orchestrate`'s
integration branch, which is the machinery we reuse.

---

## 6. Verify → findings → action (the /assess-findings mapping)

Verify output is triaged exactly like a review-findings list. The orchestrator
IS the triage step; the on-disk primitives already model each disposition:

| /assess verdict | orchestrator action | orchestratectl primitive | blocks feature? |
|---|---|---|---|
| FIX (clean) | re-code the chunk, findings in brief | new code-node iteration | yes (must converge) |
| FIX_WITH_CARE | re-code, tier promoted / narrower brief | code-node (higher tier) | yes |
| SPEC-FLAW | re-spec against intent, then re-code | spec-node iteration → plan.json update | yes |
| DISCUSS | resolve autonomously or escalate | `Discussion` projection (open→resolved) | maybe (human if genuine) |
| SPIN_OFF | record for later; do not block | `SpinoffProposal` (proposed→approved/rejected) | no (deferred) |
| DROP | note, no action | wrap_up_recommendations | no |

Because verify is never empty, the orchestrator's job is precisely to *sort the
inevitable findings* into "must-fix now (loop)" vs "defer (spin-off)" vs "not
worth it (drop)" vs "needs a human call (discuss)". This is /assess-findings run
continuously inside the build loop.

---

## 7. User-interaction points (precise, exhaustive)

Coding is **never** human-monitored (owner's rule). Every human touch is at the
design/judgment level:

1. **Feature request + intent** — entry. Human states what should exist.
2. **#1 Architecture audit (OPTIONAL)** — after spec, before code, *iff* the
   architecture is significant/uncertain. `discussion(critical)`. Default:
   auto-proceed.
3. **#2 DISCUSS escalation (CONDITIONAL)** — a verify finding needs a genuine
   human judgment the orchestrator can't make. `discussion`.
4. **#3 Intent-unreachable escalation (EXCEPTIONAL)** — repeated non-convergence
   / budget exhausted / spec can't be fixed. Orchestrator stops and asks.
5. **Spin-off backlog review (ASYNC, non-blocking)** — spun-off proposals are
   reviewed later via `/assess-findings` / `/stint`, never block the feature.
6. **Final result** — front-end receives the rollup. Autonomous self-merge by
   default (coding not monitored); an interactive variant may hold the final
   merge for a human, but that is a per-kind choice, not a code-review of the diff.

---

## 8. Scope: direct into the existing pipeline

Per owner: build this **directly into the existing coding kinds**, not as a
walled-off greenfield kind. Path:

1. Land the model-tier plumbing (task 0) + the staged supervisor + spec/code/verify
   stage roles.
2. Make the **coding** kinds (`code`, `spinoff`, `bugfix`) run through the
   spec→code→verify pipeline as their default execution model.
3. Non-coding kinds (research, technical-decision, make-skill, fan-out) unaffected.

---

## 9. Reuse vs. new infra + build order

| Reuse (exists) | New (build) |
|---|---|
| orchestrate driver, integration branch, child nodes, parent-pointer | **task 0:** `run create --model` → create.sh `--agent` (tier→agent-command config) |
| node.report → supervisor rollup, run merge, teardown | **staged supervisor** driven by `plan.json` (spec→code→verify + fix loop) |
| `Discussion` + `discussion resolve` CLI | stage roles **spec / code(implement) / verify**; `plan.json` schema |
| `SpinoffProposal` + approve/reject lifecycle | **feature-orchestrator** agent + skill (intent-holder, triage loop) |
| `/assess-findings` triage logic | **skills:** spec-brief template, code-brief template, verify template; route existing coding skills through the pipeline |
| all state-integrity invariants | adaptive tier-promotion + convergence/budget policy |

---

## 10. Open questions (to lock next)

- **Spec granularity heuristic** — when does a task warrant chunking vs a single
  code-node? Threshold owned by the spec-node; needs a rule of thumb.
- **Convergence budget** — max fix-loop iterations / token budget before
  escalation (#3). Per-feature default?
- **deepseek-flash launcher** — exact agent-command / router that makes a
  non-Claude model drive a worktree agent (does it self-merge via `run merge`
  the same way? does it honor the node.report contract?). Needs a spike.
- **plan.json schema** — concrete fields (chunk id, deps, brief, tier, verify
  criteria, files-touched). Draft before implementation.
- **Interactive final merge** — do we keep a human-gated final merge for the
  `code` kind, or is even that autonomous (arch audit being the sole gate)?
