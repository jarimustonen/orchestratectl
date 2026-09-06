---
created: 2026-07-25
updated: 2026-08-14
type: task
status: obsolete
priority: normal
related: ['@pipeline-walking-skeleton']
closed: 2026-08-14
closed_by: agent-cut-pipeline-floor-harness-heavy
---

# Wire the pipeline as a run-create coding kind (default coding path)

## Description

The T5 skeleton is a standalone additive 'pipeline run' command that keeps its own scratch state and does NOT create an taskfleet run. Per design §14 (bold-to-live, reversible rollout), wire the pipeline engine as a real coding kind behind a per-run flag (controllable, legacy engine retained for rollback), so it records events through the LockedRun + append_and_apply API (state-integrity invariant 1), is visible in 'run list/show', and is supervised/torn-down by the canonical supervisor cleanup path (invariant 5). This is the step that makes it 'how coding is done'.

## Resolution

### 2026-08-14T04:42:34Z · @agent-cut-pipeline-floor-harness-heavy

Superseded by the 0.2 subtractive cut (cut-pipeline-floor-harness-heavy): the code-pipeline subsystem (pipeline/*, floor/*) and the harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek) it targeted were deleted. Nothing left to harden/wire/triage. See docs/decisions/0001-thin-supervisor-vs-harden.md D3.
