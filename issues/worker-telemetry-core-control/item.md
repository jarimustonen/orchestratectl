---
created: 2026-08-22
updated: 2026-08-22
type: task
status: untriaged
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:core
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
---

# Implement worker telemetry control and bounded sample

## Description

Implement the approved core half of `issues/worker-telemetry-protocol/design.md`: versioned telemetry control types, attempt-scoped capability issue/revoke, retry fencing, idempotent incarnation open, strict epoch/sequence handling, and a bounded atomically replaced advisory sample.

Keep authoritative control event-projected under `LockedRun`; never reconstruct it from the disposable sample. Telemetry must not mutate status, reports, retry counters, merge state, generic attention, manifest/node progress timestamps, terminal outcomes, or cleanup eligibility. Include fake-clock, corruption, partial-write, retry/terminalization race, and negative invariant tests. This candidate remains untriaged until the Phase 1 design receives human approval.
