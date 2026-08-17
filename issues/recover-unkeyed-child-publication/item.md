---
created: 2026-08-17
updated: 2026-08-17
type: improvement
status: open
priority: high
lane: lifecycle
---

# Recover unkeyed child publication

## Description

## Description

`run create` now repairs the child-publication → parent `child.spawned` crash window on a retry that carries the same `--idempotency-key`: the published child manifest is the transaction record and the parent edge is appended idempotently by child run id.

An unkeyed child create still has no immutable operation identity a retry can use to find the already-published child. If the creator is killed after renaming the child into `runs/` but before appending `child.spawned` to the parent log, a fresh unkeyed retry creates a different child and the original remains published but undiscovered by its parent.

Decide and implement one complete contract: require `--idempotency-key` for every child create, assign an internal durable child-operation id before publication, or add a supervisor read-repair scan of child manifests keyed by an immutable transaction identity. Do not match by title/kind because repeated intentional children can share them. Preserve no-false-success and append through the parent `LockedRun` path.
