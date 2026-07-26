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
