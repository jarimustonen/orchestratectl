---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: open
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R4
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 50
blocked_by: ['@taskfleet-shared-dispatcher', '@taskfleet-dual-name-resolver', '@taskfleet-state-migration']
collision: [repository-identity]
---

# Create canonical Taskfleet packages and the bounded old CLI wrapper

## Description

## Goal

Rename active packages/layout to `taskfleet-core` and `taskfleet`, exact-pin the canonical core, and make `taskfleet` the sole canonical binary. Add an implementation-free `orchestratectl` compatibility package/binary linked to the shared dispatcher, outside layouts that could produce duplicate target artifacts. It emits stderr-only once-per-process deprecation while preserving machine stdout/JSON/JSONL and exits. Do not publish an `octl-core` wrapper absent an ADR amendment and real external dependent.

**Acceptance:** `cargo metadata`, normalized manifests, target graphs, `cargo package --list` and extracted package archives show one engine, canonical packages and one thin old wrapper with exact dependency; the wrapper is explicitly excluded from cargo-dist. Both command names pass parity/self-exec tests, including signals, logging/current-executable behavior and suppression of deprecation warnings in hidden supervisor/run-worker/retry/reattach/doctor children; wrapper metadata supports same-version 0.6/0.7 releases; doctor recognizes canonical and compatibility checkouts correctly; no GitHub/tap rename, publish, tag or install.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `50`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-shared-dispatcher`, `@taskfleet-dual-name-resolver`, `@taskfleet-state-migration`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
