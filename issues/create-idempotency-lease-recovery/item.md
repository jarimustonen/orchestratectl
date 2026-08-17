---
created: 2026-08-17
updated: 2026-08-17
type: improvement
status: open
priority: high
commits:
- hash: 44447b6
  summary: stage creates before publishing workers
---

# Recover interrupted run-create reservations and child publication

## Description

Follow-up from pi-spinoff-batch review. Implement a durable creator lease/owner identity for pre-publication idempotency reservations so a retry can distinguish a live materializer from a dead one and safely reclaim stale staging state. Also make child publication plus parent child.spawned recoverable across their two event logs, with read repair or a recorded transaction. Preserve no-false-success semantics and add deterministic crash/retry tests.
