---
created: 2026-07-25
updated: 2026-07-25
type: task
status: done
priority: normal
related: ['@pipeline-walking-skeleton']
closed: 2026-07-25
---

# Pipeline fix loop: RE_CODE_CHUNK + TRIGGER_RE_SPEC on floor/verify failure

## Description

The T5 walking skeleton (`pipeline run`) stops on the first chunk or feature that fails the floor/verify (v1 has no fix loop). Wire the deferred verify→triage→fix cycle: on a floor-blocked chunk or a failed verify, feed findings back as RE_CODE_CHUNK (re-brief + re-run the chunk, must re-verify before close, design §8), and on a SPEC-FLAW emit TRIGGER_RE_SPEC (new plan.v(N+1) + DAG-diff to decide which chunks revert to Pending, design §7). Reuse the T4 driver's Action primitives + envelopes. Bound by circuit-breakers (see pipeline-circuit-breakers), never by judgment alone.
