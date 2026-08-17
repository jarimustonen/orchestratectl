---
created: 2026-08-17
updated: 2026-08-17
type: improvement
status: in-progress
priority: high
commits:
- hash: 44447b6
  summary: stage creates before publishing workers
- hash: 30f31a3
  summary: stage creates before publishing workers
- hash: 70e21ad
  summary: mark create lease recovery in progress
- hash: 177be48
  summary: recover interrupted create reservations
- hash: ddaa1a9
  summary: harden create lease recovery after multi-model review
lane: lifecycle
lane_seq: 5
---

# Recover interrupted run-create reservations and child publication

## Description

Follow-up from pi-spinoff-batch review. Implement a durable creator lease/owner identity for pre-publication idempotency reservations so a retry can distinguish a live materializer from a dead one and safely reclaim stale staging state. Also make child publication plus parent child.spawned recoverable across their two event logs, with read repair or a recorded transaction. Preserve no-false-success semantics and add deterministic crash/retry tests.

## Comments

### 2026-08-17T08:15:25Z · @orchestrator

Re-laned supervise→lifecycle 2026-08-17. The `supervise` lane was created for pi-spinoff-batch on the prediction that it would touch supervise/*; git shows its fix landed ENTIRELY in crates/octl-cli/src/run/create.rs (247 lines) with zero files under supervise/. This follow-up inherits that surface — pre-publication idempotency reservations and child publication are run-create plus the octl-core event log. Keeping a separate `supervise` lane asserted disjointness from `lifecycle` that does not exist, and would have permitted a parallel spawn colliding on run/create.rs. Same shape as the two integrated-main breakages recorded in TODO.md.
