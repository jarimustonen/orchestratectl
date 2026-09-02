---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: open
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R3
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 40
blocked_by: ['@taskfleet-dual-name-resolver']
collision: [repository-identity]
---

# Implement quiescent same-filesystem Taskfleet state migration

## Description

## Goal

Add dry-run and explicit migration commands. Require exact normalized source/destination, absent destination, external migration lock, same filesystem and quiescence: no non-terminal run, live supervisor/worker, pending merge, held run lock or state-writing command. Validate runs through normal lock/reducer APIs, atomically rename the whole root, write/verify the outside receipt and leave no symlink/alias. Permanently fail on recreated/dual roots. Define first canonical write and permit rename-back only before it.

**Acceptance:** define an outside receipt location/state machine, durable ordering/fsync and fail-closed recovery; add bounded/nonblocking per-run lock checks and state explicitly that future locks cannot fence every old 0.5.1 process or open descriptor, so operator-enforced exclusion is required where automatic proof is impossible. Migration logging stays outside source/destination until resolution; log creation is an explicit first-canonical-write boundary. Fixture event hashes, `applied_seq`, ids, OIDs, branches and pending transaction semantics survive. Runtime builders cover active/stale old processes, open descriptors, pending merge, dual roots, destination, symlink/path, held lock, crash points, receipt faults and cross-device refusal; rollback boundary is tested; no public identity mutation.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `40`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-dual-name-resolver`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
