---
created: 2026-07-03
updated: 2026-08-16
type: improvement
status: open
priority: normal
related: ['@supervisor-dead-merge-no-teardown']
labels: [defer-0.2.1]
lane: lifecycle
lane_seq: 90
---

# Extend dead-supervisor liveness recovery to run cancel

_Source: crates/octl-cli/src/run/cancel.rs_

## Description

Follow-up spinoff from supervisor-dead-merge-no-teardown (fix 979b794/62948c8). run cancel synthesizes terminal node.reports and, like the old run merge, relies on a live supervisor to roll the run up and tear down — same orphaning failure mode when the supervisor is dead. The recovery is now a reusable helper (SupervisorView::probe + reattach::spawn_supervisor + the ensure_report_consumer pattern), so applying the same treatment to cancel is a small, well-scoped follow-up. Left out of the merge fix to keep that correctness-sensitive change tightly scoped and separately reviewable. Non-blocking for v0.1.0.

## Decisions

### 2026-08-13T11:10:30Z · @adr-decision-2

DEFER-to-0.2.1: Dead-supervisor recovery is the lease's job. The clean answer is the pi.dev self-report/lease plugin (0.2.1), not the 0.2.0 thin core. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
