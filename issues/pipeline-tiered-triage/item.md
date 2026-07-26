---
created: 2026-07-25
updated: 2026-07-26
type: task
status: in-progress
priority: normal
related: ['@pipeline-walking-skeleton']
---

# Pipeline adaptive tier promotion + tiered fast-coordinator triage

## Description

The T5 skeleton runs every chunk at its plan-declared tier and makes each spec/verify call directly on Opus. Wire the design §0.2/§2 tiered orchestrator into the live loop: a fast, cheap coordinator classifies each decision routine-vs-consequential and emits routine primitives directly, deferring consequential ones (DECLARE_CONVERGED, TRIGGER_RE_SPEC, ESCALATE, non-trivial spinoff) to the Opus decider; and PROMOTE_TIER re-runs a chunk at a higher tier on repeat-fail or verify self-disagreement (design §3). Reuse TieredOrchestrator + DecisionEnvelope::validate_for from the T4 scaffold.

## Comments

### 2026-07-26T09:33:37Z · @claude

T6 landed: fast-coordinator/decider routing seam (route_proposal reused by TieredOrchestrator + live loop) and adaptive PROMOTE_TIER on repeat-fail (tier ladder code→mid→high via TierHarness, bounded by max_promotions). Routine decisions (RE_CODE, PROMOTE) never hit the decider; consequential (DECLARE_CONVERGED, TRIGGER_RE_SPEC) defer to it and honour an ESCALATE override. Passed multi-model /llm-review; confirmed fixes applied (monotonic attempt-seq preventing promoted-branch collisions, resolver-driven promotion availability, decider-verdict execution for re-spec, validate_for assert).

DEFERRED (issue kept open): (1) verify SELF-DISAGREEMENT as a promotion trigger — design §3/§8 adversarial two-pass verify (find-bugs vs confirm-it-ships) not wired, so promotion fires only on repeat-fail; needs a VerifyProvider disagreement signal (cost-sensitive, overlaps pipeline-circuit-breakers). (2) LiveDecider is a confirming provenance-seam, not a distinct second-opinion Opus call — a real ClaudeDecider is a follow-up. (3) DecisionTrigger has no ChunkFailed/VerifyPassed variant, so the live loop reuses ChunkCommitted/VerifyReport for the decider context.
