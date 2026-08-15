---
created: 2026-08-15
updated: 2026-08-15
type: task
status: done
priority: high
epic: lifecycle-architecture-review
commits:
- hash: 0795ad4
  summary: 'feat(merge): deterministic OID-based recovery for run merge transactions (A2)'
- hash: ab59f55
  summary: 'fix(merge): harden A2 recovery per /llm-review + /assess-findings'
- hash: ac402d4
  summary: record A2 recovery commits on issue
closed: 2026-08-15
---

# Thin supervisor: recover run merge transactions by OID

## Description

## Goal
Implement the merge-transaction recovery from `issues/lifecycle-architecture-review/design.md` §2.1b (A2): `run merge` records a narrow transaction before mutating git, then completes or rejects that known transaction after crashes.

## Context
The thin model removes the broad git-reconcile-implies-done heuristic, but `run merge` spans two durability domains: git refs and the event log. A crash after the source branch update but before the explicit-merge event must not become a false failure. Record `merge.started{op_id, expected_source_oid, worker_oid}` before mutation and finish by exact OID on the next lock acquisition.

Likely touches `crates/octl-core/src/{events,reducer,schema}.rs`, `crates/octl-cli/src/run/merge.rs`, and supervisor recovery code.

## Done criteria
- `run merge` writes a durable merge-start record before git mutation.
- Recovery is deterministic and OID-based, not a general branch heuristic.
- Crash-window tests cover git-mutated/event-not-yet-appended and no-mutation cases.
- Full project green gate passes, including docs.
