---
created: 2026-07-23
updated: 2026-07-23
type: task
status: in-progress
priority: high
---

# T4: inverted control loop — supervisor owns loop, tiered orchestrator (fast coordinator + Opus decider), typed action primitives + decision envelopes (scaffold, behind seam)

## Description

Builds the architectural core of the code-pipeline epic (design.md §2 + §0.2) as
a NEW, isolated module `crates/octl-cli/src/pipeline/`, behind the seam — no live
supervisor / `run create` / merge path touched. T5 later wires it into the real
supervisor + event log.

Delivered:
- **Typed action primitives** (`action.rs`): `Action` enum — `ReCodeChunk`,
  `TriggerReSpec`, `AcceptChunk`, `PromoteTier`, `OpenDiscussion`,
  `ProposeSpinoff`, `DeclareConverged`, `Escalate` — serde-tagged, never prose.
- **Classification table** (`Action::decision_class`): `Routine` vs
  `Consequential`, encoded explicitly + tested. Spin-off triviality carried by a
  dedicated `SpinoffScope`, not overloaded onto finding `Severity`.
- **Decision envelope** (`envelope.rs`): `DecisionEnvelope` (actor, input
  artifacts, reason, `decision_tier`, model, prompt version) + the tier invariant
  `validate_for` — a consequential action stamped `coordinator` is a caught
  `TierViolation`.
- **Tiered orchestrator** (`orchestrator.rs`): `Orchestrator` trait + stateless
  `DecisionContext` (with a read-only chunk snapshot); `TieredOrchestrator<C, D>`
  routes consequential proposals from a fast `Coordinator` to an expensive
  `Decider`, stamping the tier by construction. Deterministic scripted stubs.
- **Loop skeleton** (`driver.rs`): `drive` — a pure in-memory state machine.
  Atomic `DecisionRecord { action, envelope, outcome }` audit trail; fail-closed
  on tier violation + circuit-breaker (deterministic escalate, no LLM); chunk
  preconditions checked before would-execute; stubbed `ActionExecutor`.
- **Tests**: 30 unit tests covering the routine FIX loop, decider-tier
  `DeclareConverged`/`Escalate`/`TriggerReSpec`, mis-tier rejection, superseded
  post-terminal actions, unknown-chunk rejection, promote/re-spec transitions,
  serde round-trips. No LLM/network.

Reviewed with `/llm-review` (Gemini 3.1 Pro, GPT-5.6-sol, Opus 4.7); real
findings addressed (atomic decision record, fail-closed posture, deterministic
breaker, `SpinoffScope`, precondition ordering). Report in
`history/review-pipeline-t4-control-loop.md`. A design under-specification found
during the build (DROP primitive, §2 vs §8) filed as
`pipeline-drop-primitive-underspecified`.

