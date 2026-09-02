---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: untriaged
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R1
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
---

# Extract the shared Taskfleet CLI dispatcher

## Description

## Goal

Refactor the current binary entry point into one linkable dispatcher used later by the canonical Taskfleet binary and bounded old CLI wrapper. Keep parser, execution, envelopes, state resolution and error formatting shared. Add explicit invocation identity only where help/version/deprecation require it. Hidden self-exec paths (`supervise`, `run-worker`, merge/recovery, reattach and doctor fix) must use the current executable/shared entry point, never a PATH lookup or a second engine.

**Acceptance:** current `orchestratectl` stdout/JSON/JSONL/exit behavior is unchanged; one dispatcher owns command execution and takes explicit invocation identity without unsafe PATH or argv-name inference; self-exec tests cover every hidden path; full Rust gate and snapshots pass. No package, binary, home, repository or distribution rename occurs.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `20`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-rename-inventory`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
