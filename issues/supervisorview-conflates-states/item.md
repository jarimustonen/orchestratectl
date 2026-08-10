---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: open
priority: normal
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
