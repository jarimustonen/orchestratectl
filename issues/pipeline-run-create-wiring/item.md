---
created: 2026-07-25
updated: 2026-07-25
type: task
status: open
priority: normal
related: ['@pipeline-walking-skeleton']
---

# Wire the pipeline as a run-create coding kind (default coding path)

## Description

The T5 skeleton is a standalone additive 'pipeline run' command that keeps its own scratch state and does NOT create an orchestratectl run. Per design §14 (bold-to-live, reversible rollout), wire the pipeline engine as a real coding kind behind a per-run flag (controllable, legacy engine retained for rollback), so it records events through the LockedRun + append_and_apply API (state-integrity invariant 1), is visible in 'run list/show', and is supervised/torn-down by the canonical supervisor cleanup path (invariant 5). This is the step that makes it 'how coding is done'.
