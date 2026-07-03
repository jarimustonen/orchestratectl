---
created: 2026-07-03
updated: 2026-07-03
type: improvement
status: open
priority: normal
related: ['@supervisor-dead-merge-no-teardown']
---

# Extend dead-supervisor liveness recovery to run cancel

_Source: crates/octl-cli/src/run/cancel.rs_

## Description

Follow-up spinoff from supervisor-dead-merge-no-teardown (fix 979b794/62948c8). run cancel synthesizes terminal node.reports and, like the old run merge, relies on a live supervisor to roll the run up and tear down — same orphaning failure mode when the supervisor is dead. The recovery is now a reusable helper (SupervisorView::probe + reattach::spawn_supervisor + the ensure_report_consumer pattern), so applying the same treatment to cancel is a small, well-scoped follow-up. Left out of the merge fix to keep that correctness-sensitive change tightly scoped and separately reviewable. Non-blocking for v0.1.0.
