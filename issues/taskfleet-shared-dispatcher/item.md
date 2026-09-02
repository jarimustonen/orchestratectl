---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: done
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R1
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 20
blocked_by: ['@taskfleet-rename-inventory']
collision: [repository-identity]
closed: 2026-09-02
commits:
- hash: c19fc6de503b908f107892a13a7a74dc1d15cfb7
  summary: extract shared invocation dispatcher
---

# Extract the shared Taskfleet CLI dispatcher

## Description

## Goal

Refactor the current binary entry point into one linkable dispatcher used later by the canonical Taskfleet binary and bounded old CLI wrapper. Keep parser, execution, envelopes, state resolution and error formatting shared. Add explicit invocation identity only where help/version/deprecation require it. Hidden self-exec paths (`supervise`, `run-worker`, merge/recovery, reattach and doctor fix) must use the current executable/shared entry point, never a PATH lookup or a second engine.

**Acceptance:** current `orchestratectl` stdout/JSON/JSONL/exit behavior is unchanged; one dispatcher owns command execution and takes explicit invocation identity without unsafe PATH or argv-name inference; self-exec tests cover every hidden path; full Rust gate and snapshots pass. No package, binary, home, repository or distribution rename occurs.

## Acceptance Criteria

- [x] Existing `orchestratectl` text, JSON, JSONL, and exit behavior remains compatible.
- [x] One linkable dispatcher owns execution and takes explicit invocation identity.
- [x] Hidden self-exec uses the exact current executable without PATH or argv-name inference.
- [x] Hostile/stripped-PATH and hidden-path tests pass.
- [x] LLM review is assessed and the complete Rust gate and snapshots pass.
- [x] No package, binary, home, repository, or distribution rename is included.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `20`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-rename-inventory`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.

## Resolution

### 2026-09-02T10:45:26Z · @issuectl

Implemented ADR 0002 R1. LLM review findings were assessed and resolved; the complete Rust gate passed, including 1,088 release tests, doctests, rustdoc, clippy, formatting, snapshots, and the identity-inventory verifier.
