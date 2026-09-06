# Code Pipeline — design (v2, post-panel)

Spec-driven, model-tiered coding as the **default coding path** in
taskfleet. Expensive thinking is done once (spec) and amortized across
cheap implementation (code); a heavy model verifies **on top of a deterministic
floor**; findings loop back until the product matches the *original intent*.

Status: DESIGN v2 — folds in the 2026-07-22 multi-model panel non-negotiables
(`history/2026-07-22-panel-code-pipeline.md`). Implementation-ready pending the
three owner decisions in §15 (defaults applied, flagged for override).

---

## 0. Design principles (revised)

1. **Trust model judgment — on the quality axis only (RE-SCOPED).** Judgment
   governs *quality and sizing*: chunk size, must-fix vs nice-to-have, whether a
   re-verify is warranted. It does **not** govern the *correctness-gate axis* or
   the *resource-safety axis* — those are deterministic and mechanical (§4, §9).
   No brittle numeric rules for sizing; hard mechanical rules for "did tests pass"
   and "did we blow the budget." Different axes. *(Amends the v1 principle per the
   panel; see §15 decision D1.)*
2. **Concentrate FINAL decisions in the expensive model — routine coordination may
   be fast.** Every *final/consequential* judgment — declare-converged / ship,
   trigger-re-spec, accept-a-risk or drop a non-trivial finding, escalate-to-human —
   is made by Opus. The project-manager / coordinator tier (dispatching chunks,
   tracking progress, sorting obvious findings) **may run on a fast, cheap model** so
   the always-on layer is cheap. Cheap *code* models still make no decisions. Every
   decision envelope records **which tier decided**; a fast-model *final* decision is
   an audit-catchable bug. The deterministic floor (§4) is what makes a cheap
   coordinator safe — the mechanical gates don't care which model coordinated.
   *(Owner refinement, 2026-07-23.)*
3. **Single human-interaction locus.** All human conversation flows through one
   front-end agent; sub-agents escalate *up*, never address the human. No dumb
   human-gated merge. (A *passive, non-blocking* post-merge rollup is allowed —
   §12, decision D2.)
4. **Self-improving — but governed — tooling.** An agent that hits a contract
   limitation files a **structured schema-gap** into the taskfleet repo. That
   is a *deduplicated, evidence-backed proposal for human review*, NOT
   "agent asks → field added." Runtime agents never act on undeclared fields (§13).
5. **Harness-neutral by contract.** The code executor is chosen behind a versioned
   adapter interface with a conformance suite; no runtime is crowned by a single
   spike (§10).
6. **Reversible rollout.** The pipeline becomes the default *end state* through
   staged, per-run-configurable deployment with a retained legacy engine — never a
   big-bang flip (§14).

---

## 1. The invariant: intent (a first-class, orchestrator-owned artifact)

The anchor is the **original intent** — what must exist. The spec is only a draft
of it. Convergence = "product matches intent," never "spec implemented verbatim"
nor "verify came back clean."

**Intent is a separately versioned, orchestrator-owned artifact** (`intent.md` +
an `intent_rev`), NOT a field the spec node can rewrite. The plan *references* the
intent revision it was built against. A spec update (living draft) produces a new
plan revision but **cannot silently redefine intent** — weakening intent or
dropping acceptance criteria requires a logged, auditable orchestrator rationale
(guards the "goalpost-mover" failure the panel flagged).

**Verify is almost never "ok"**, so the verify→triage→fix loop is the *normal*
cycle. It is kept short by judgment (orchestrator estimates change scope; ~2
rounds is already a lot → escalate, don't grind) — but bounded hard by the
resource circuit-breakers of §9, never by judgment alone.

---

## 2. Architecture: inverted control loop

The panel's load-bearing correction. The non-LLM **supervisor owns the event
loop**; the Opus **orchestrator is a stateless pure function**, invoked per
decision point and returning **discrete typed action primitives** — never natural
language, never a long-running LLM driver (which would exhaust context and
hallucinate state transitions).

```
Supervisor (event-sourced state machine, owns the loop)
  │  at each decision point, calls the orchestrator as a pure function:
  │      Triage(verify_report, plan_rev, intent_rev)  ->  Action[]
  ▼
Orchestrator [Opus, stateless]  returns e.g.:
      RE_CODE_CHUNK(id, findings)          TRIGGER_RE_SPEC(reason, chunk_ids)
      ACCEPT_CHUNK(id)                     PROMOTE_TIER(id, tier)
      OPEN_DISCUSSION(topic, severity)     PROPOSE_SPINOFF(...)
      DECLARE_CONVERGED()                  ESCALATE(reason)
```

The supervisor validates and executes each primitive, appends events, and only
re-invokes the orchestrator when the next decision is due. Decisions are recorded
as **structured envelopes** (actor, input artifact IDs, reason summary, **decision
tier**, model + prompt version), not prose — so a run is causally replayable.

**The orchestrator function is itself tiered (owner refinement §0.2).** It runs on
a **fast, cheap model** for routine coordination — emitting the obvious mechanical
primitives (`RE_CODE_CHUNK` for a clear FIX, dispatch, progress). It **must route
final/consequential primitives to the expensive model**: `DECLARE_CONVERGED`,
`TRIGGER_RE_SPEC`, `ESCALATE`, and any DROP/`PROPOSE_SPINOFF` of a non-trivial
finding. Concretely: the fast coordinator classifies each decision as routine vs
consequential; consequential ones are deferred to an Opus call whose verdict is the
one recorded. The classification boundary is the one genuinely new risk this
refinement adds — a fast model that mislabels a consequential decision as routine —
so the envelope's `decision_tier` field makes every such call auditable, and the
deterministic floor (§4) still gates the merge regardless of who coordinated.

---

## 3. Roles & model tiers

| Role | Model (v1) | Decisions? | Responsibility |
|---|---|---|---|
| **front-end** | Opus (user's convo) | routes to human | the sole human locus; spawns one feature-orchestrator per feature |
| **coordinator (PM)** | **fast/cheap, stateless fn** | routine only | dispatch + obvious-triage → typed primitives; classifies each decision routine vs consequential; state lives in the log |
| **decider** | **Opus** | **final/consequential only** | invoked by the coordinator for ship/converge, re-spec, escalate, non-trivial drop/spinoff; its verdict is the recorded one |
| **spec** | Opus | yes (arch) | writes `plan.vN.json` (chunk DAG + turnkey briefs + checks/assertions); chunked by judgment, no numeric rules |
| **code** | deepseek (cheap) | **no** | reads a brief, edits, runs self-check, commits its chunk branch; never merges, never reports (supervisor synthesizes the result) |
| **verify** | Opus | yes (must-fix) | runs *above the deterministic floor*; emits findings as node.report assets |
| **supervisor** | — (code) | none | owns the loop; enforces the floor; merges chunks; circuit-breakers; teardown |

Code tier is a **pluggable adapter binding** (§10). Adaptive promotion: a chunk
that fails verify **or on which verify disagrees with itself** is re-run at a
higher tier.

---

## 4. The deterministic floor (NEW — the panel's #1 demand)

LLM verify is **advisory on top of a mechanical floor**, never the gate itself.
The **supervisor** (not verify) enforces, against a **baseline snapshot captured
at `feat/<slug>` fork** (test pass-list hash, clippy-warning-list, optional
coverage):

- **No feature merges to `source_branch`, and no chunk merges to `feat/<slug>`, unless:**
  1. the relevant checks pass (fast per-chunk; full suite at feature tip);
  2. **no test that passed at the baseline is now failing**;
  3. **no new clippy warnings** vs. baseline;
  4. **no test-suite gaming**: test count didn't drop, none newly `#[ignore]`/skipped/renamed-to-no-op, assertion density in touched files didn't regress.
- **File-scope is a merge-time constraint** (not just a hint): the supervisor
  rejects a chunk merge whose `git diff --name-only` exceeds `files_touched[]`
  beyond a configured slack. Out-of-scope edits force escalate/re-spec. (Execution
  stays unconstrained; the guard is at the boundary — injection-resistant.)

`plan.json` therefore splits criteria into **executable `checks`** (a test path +
name, a shell command) and **LLM-judged `assertions`**. Every chunk has ≥1 check;
`acceptance[]` has ≥1 executable end-to-end check. **Test-authoring is mandatory**
— either a dedicated stage or an explicit, supervisor-verified part of every code
chunk's brief ("behavior chunk committed without new/modified tests" = merge
blocker). This gives the autonomous loop a ground-truth oracle instead of two LLMs
reading English at each other.

---

## 5. Contexts and who spawns whom

```
USER ──only human touchpoint──► FRONT-END [Opus]  (the human's conversation)
                                   │ spawns one feature-orchestrator per feature
                                   ▼
                          FEATURE-ORCHESTRATOR [Opus, stateless fn]
                                   │ returns typed actions to ↓
                                   ▼
                          SUPERVISOR (owns loop; enforces §4 floor; §9 breakers)
                             ├─ spec   [Opus]        → plan.vN.json + intent ref
                             ├─ code×N [deepseek]    → commit chunk branch (no merge)
                             └─ verify [Opus]        → findings (above the floor)
```

One orchestrator = one feature (one intent, one integration branch). Multi-feature
concurrency = the front-end spawns several single-feature orchestrators in
parallel. No multiplexing orchestrator.

---

## 6. Call diagram (v2)

```
 user ──"feature X + intent"──► FRONT-END [Opus]         ◄── only human touchpoint
                                   │ create feature-orchestrator run
                                   │   ↳ fork feat/<slug> off source; SNAPSHOT BASELINE (§4)
                                   │   ↳ write intent.md (orchestrator-owned, versioned)
                                   ▼
                            SUPERVISOR owns the loop ───────────────────────────────┐
   VAIHE 1  spawn spec [Opus] → plan.v1.json (DAG, turnkey briefs, checks+assertions)│
            (targeted context, NOT whole-repo — §11; chunked by judgment)           │
            ⟶ orchestrator Triage: proceed | (opt) escalate arch question UP         │
   VAIHE 2  spawn code-nodes per DAG [deepseek]: edit → self-check → commit chunk    │
            SUPERVISOR merges each chunk → feat/<slug> ONLY IF §4 floor holds        │
            (merge conflict → deterministic protocol: re-spawn "rebase&fix" or       │
             escalate; sequential chunks stack on the moved tip)                     │
   VAIHE 3  spawn verify [Opus] on feat/<slug>: floor already green; verify adds     │
            judgment. Findings = fix_items / discussion / spinoff / drop (+dismissed) │
   ┌────────  orchestrator Triage(report) → Action[]  (loop)                         │
   │ RE_CODE_CHUNK   → re-brief the chunk; FIX-class MUST be re-verified before close │
   │ TRIGGER_RE_SPEC → new plan.v(N+1); DAG-diff decides which chunks revert PENDING  │
   │ OPEN_DISCUSSION → bubbles UP to front-end → human only if it needs a person      │
   │ PROPOSE_SPINOFF → deferred backlog (non-blocking)                                │
   │ PROMOTE_TIER    → on repeat-fail OR verify self-disagreement                     │
   └─ DECLARE_CONVERGED  (no must-fix left AND product matches intent)                │
            │  ▓ circuit-breakers (§9) can force ESCALATE at any point ▓              │
   VAIHE 4  supervisor merges feat/<slug> → source (AUTOMATIC, floor re-checked);     │
            passive post-merge rollup surfaced to front-end (§12); branch dies        │
                                   └───────────────────────────────────────────────┘
```

---

## 7. Integration branch + plan revisions (lifecycle)

| Phase | Event |
|---|---|
| run created | fork `feat/<slug>` off `source_branch`; **capture baseline snapshot** (§4); write `intent.md` |
| spec | spec-node writes **immutable** `plan.v1.json` referencing `intent_rev` |
| code | chunk forks off current `feat/<slug>`; code commits; **supervisor merges iff floor holds**; sequential chunks stack |
| verify | runs on tip, above the green floor |
| fix | `RE_CODE_CHUNK` re-commits on `feat/<slug>` |
| re-spec | spec-node writes **`plan.v(N+1).json`** (never overwrites); supervisor DAG-diffs vN→v(N+1) → which chunks revert to PENDING, which stay DONE; each chunk attempt records the exact plan_rev it consumed |
| converged | automatic merge `feat/<slug>` → `source_branch` (floor re-checked) |
| teardown | supervisor removes branch + chunk worktrees/tmux; baseline + plan revisions retained in run dir for audit |

Plans are **immutable and content-addressable per revision**; provenance
(intent_rev, plan_rev, model, harness, prompt version) is recorded on every chunk
attempt and verify report.

---

## 8. Verify → findings → action

Triaged like `/assess-findings`, but the floor is mechanical and below it.

| verdict | typed action | primitive | blocks? |
|---|---|---|---|
| FIX / FIX_WITH_CARE | re-code (findings in brief); **must re-verify before close** | `RE_CODE_CHUNK` | yes |
| SPEC-FLAW | new plan revision against intent, then re-code affected | `TRIGGER_RE_SPEC` | yes |
| DISCUSS | resolve, else escalate UP | `OPEN_DISCUSSION` | maybe |
| SPIN_OFF | defer (non-blocking) | `PROPOSE_SPINOFF` | no |
| DROP | record **with rationale** | envelope | no |

Anti-sycophancy: **dismissed findings are recorded with rationale** (triage is
auditable since no human watches); verify may be run **adversarially** (a
"find-bugs" pass + a "confirm-it-ships" pass — disagreement ⇒ escalate); tier is
promoted on verify self-disagreement, not only on repeat failure. Cheap-model
output (comments, test names, docstrings) is treated as **untrusted** — the
mechanical floor is injection-resistant; the LLM layer must not be steered by
artifacts in the diff.

---

## 9. Resource circuit-breakers (NEW)

Distinct from quality judgment (principle 1). Deterministic, supervisor-owned,
per-feature ceilings that force `ESCALATE`/abort regardless of convergence state:

- **cost/token ceiling** — a hard cap (target: total feature cost ≤ ~2× the
  all-Opus cost); a **cost kill-switch** on breach.
- **wall-time**, **process-count**, **storage** ceilings.
- **repeated-identical-failure** breaker (same chunk failing the same check N times
  → stop, don't loop).

Requires **cost instrumentation**: the orchestrator must be able to query
real-time spend/token consumption per run. (Open: does taskfleet meter usage
per node today, or must it be built — §15.)

---

## 10. Harness = adapter interface + conformance suite (NEW)

Do **not** pick one global executable. Define a versioned, harness-neutral
code-node contract *before* choosing A/B/C:

```rust
trait CodeHarness {
    fn capabilities(&self) -> HarnessCapabilities;
    fn run_chunk(&self, req: ChunkRequest) -> Result<ChunkResult, HarnessError>;
}
// ChunkRequest: run/chunk/attempt ids, worktree, base_commit, plan_rev, brief, checks
// ChunkResult: outcome, resulting_commit, changed_files, check_results, transcript_ref, usage
```

The supervisor consumes only `ChunkResult` — it **never parses tool-specific prose
or infers success from exit status**. A **conformance suite** tests each adapter
against: clean success, no-change, partial-edit-then-fail, self-check fail,
timeout/cancel, malformed output, unexpected extra commits, dirty worktree,
provider failure, transcript+usage capture.

- **Primary (option A — ALREADY BUILT): `claude-deepseek`.** The code-node is the
  **same Claude Code agent** pointed at a deepseek backend via
  `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic` — the existing wrapper
  `~/bin/claude-deepseek` (homebase dotfiles). It natively speaks node.report +
  skills (it IS Claude Code), so no second runtime and no router to build. Flags:
  `--model flash|pro` (flash = `deepseek-v4-flash[1m]`; tier map opus→pro,
  sonnet/haiku→flash), `--gertrud` for a local ds4-server (no API cost, Tailscale).
  This is the first live binding for the code-node.
- **pi.dev — candidate (customizable, multi-provider).** `earendil-works/pi`, a
  terminal coding-agent harness with a unified multi-provider model API and
  TypeScript extensions. Stronger customization / provider flexibility; a good
  future adapter if we outgrow claude-deepseek. Evaluate deliberately when that need
  appears, not now.
- **aider — dropped** from the plan (was a feasibility probe only; claude-deepseek
  supersedes it as the same-runtime option A).
- The `CodeHarness` seam (T0) still holds: `claude-deepseek` and pi.dev are both
  adapters behind it, so the harness stays swappable.

Credentials/routing config stay **out of `plan.json`**; the runtime binding
(harness + concrete model + params) is recorded in execution events.

---

## 11. Cost model + token discipline

Spend is Opus-dominated (orchestrator + spec + verify); the cheap coder is noise.
Amortization holds **only if Opus token use is bounded**:

- **spec and verify must be token-efficient** — NOT whole-repo reads. Use targeted
  file selection / diff-scoping / symbol index / code maps. Verify inspects the
  branch diff + touched files, not the world.
- **Trivial-task handling (decision D3, default applied):** the pipeline stays the
  default, but for a task the orchestrator judges trivial the stages **collapse
  gracefully** — spec emits a single-chunk plan, verify is tightly scoped — rather
  than a separate "skip the pipeline" path. "Always the pipeline" holds
  structurally; cost is bounded by collapse, not by an escape hatch. (Owner may
  instead want a hard escape — §15.)

---

## 12. Human interaction (single locus + passive visibility)

The human talks only to the front-end. Escalation chain:
`verify → orchestrator → front-end → human`. No sub-agent addresses the human; no
human-gated merge.

**Passive post-merge rollup (decision D2, default applied):** on automatic merge
the front-end receives a **non-blocking** summary — what shipped, acceptance-check
results, and **dismissed/deferred findings with rationale**. It is visibility, not
a gate; it does not pause the merge. Satisfies the test-strategist's audit need
without violating the single-locus / no-dumb-gate rule. (Owner may drop it — §15.)

---

## 13. `plan.json` v2 + governed schema evolution

See `plan-schema.md` (updated to v2): adds `checks` vs `assertions`,
`baseline` reference, `intent_rev`, immutable `plan_rev`, and per-chunk provenance.

Schema evolution is **governed** (principle 4): a runtime gap emits a *structured
schema-gap event* → deduplicated candidate issue (occurrence count, affected runs)
→ human contract review → schema decision record → versioned schema + migration +
reader/writer conformance tests. Readers **validate against a checked-in schema and
reject unsupported major versions / undeclared required fields** — tolerant reading
is limited to genuinely additive optional fields.

---

## 14. Rollout — bold to live (owner decision 2026-07-24)

The owner chose to go **boldly live**, not through a long shadow/canary ceremony.
This is safe *because the deterministic floor (§4) mechanically gates every merge* —
the floor + `/llm-review` are the guardrails that let us skip staged caution. We
still keep two cheap insurances:

1. Land the **harness seam + observability + provenance** (done/ongoing — T0/T10).
2. Wire the pipeline engine as the **real coding path**, usable for real work soon —
   behind a **per-run flag** (so it's controllable and a bad run can't take the
   default path hostage) with the **legacy engine retained for rollback**. No long
   shadow period, no canary gating.

**Honest residual risk:** bold-live removes the staging safety nets the panel
recommended; the floor and review are therefore the *only* nets and must not fail.
The floor is non-negotiable and blocks any merge that regresses tests/clippy vs
baseline or escapes file-scope — so a bad autonomous decision cannot silently ship.

"Always how coding is done" is reached directly, with the floor as the hard gate.

---

## 15. Owner decisions (defaults applied — flag to override)

- **D1 — Principle 1 re-scope (§0.1):** applied. Judgment governs quality/sizing;
  correctness + resource are deterministic. *Confirm or override.*
- **D2 — Passive post-merge rollup (§12):** applied (non-blocking visibility).
  *Keep or drop.*
- **D3 — Trivial-task handling (§11):** applied as *graceful stage collapse*
  (pipeline always runs, collapses for tiny tasks) rather than a hard escape hatch.
  *Confirm, or switch to a hard skip-spec escape.*

Plus non-blocking unknowns to lock during build: does taskfleet meter
per-node token/cost today (§9); exact `checks`/`assertions` schema shape (§13);
sequential-chunk git mechanics (stacked branches vs worktrees, §7).

---

## 16. Build order

See `breakdown.md` for the sequenced task plan. Critical path starts at
**task 0: the `CodeHarness` adapter interface + result protocol + conformance
suite** (everything else consumes it), with the router qualified as one adapter —
not as a prerequisite.
