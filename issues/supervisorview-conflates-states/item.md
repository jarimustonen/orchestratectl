---
created: 2026-08-10
updated: 2026-08-11
type: improvement
status: done
priority: normal
commits:
- hash: e0f8594
  summary: distinguish SupervisorView states (alive/dead/not-recorded/unreadable)
- hash: 927e4cf
  summary: close probe TOCTOU + stop indeterminate states driving stall verdicts (llm-review F1/F2/F3)
- hash: 62ae6a5
  summary: refresh version envelope snapshots to 0.1.5 (unblock green gate)
closed: 2026-08-11
---

# run show/list: SupervisorView conflates absent/dead/unreadable/unprobed states

## Description

`SupervisorView` (used by both `run show` and `run list`) collapses several distinct
states into a single `{pid: null, alive: false}`: supervisor never launched, supervisor
exited cleanly (pid file removed), pid file present-but-unreadable, and "not probed". The
`probe` path also swallows I/O errors (`read_pid_record` returns `Option`). A consumer
reasoning about `alive == false` cannot tell "orphaned" from "finished cleanly" from "IO
error", which risks a wrong `run reattach`/`run cancel` decision.

Proposed: a wire-level `SupervisorState` enum (e.g. `alive | dead | not-recorded |
unknown`) replacing the boolean, with a migration/back-compat plan (it touches every
`SupervisorView` consumer, the run show/list snapshots, and skill docs). Raised by the
llm-review panel on `run-show-json-null-fields`; deferred there as its own design.
