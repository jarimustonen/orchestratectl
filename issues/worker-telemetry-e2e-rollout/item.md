---
created: 2026-08-22
updated: 2026-08-22
type: task
status: untriaged
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:e2e
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
---

# Validate and roll out worker telemetry end to end

## Description

Validate and roll out the approved worker telemetry protocol end to end after both orchestratectl and the external pi adapter exist.

Install a pinned adapter only in an isolated test environment and execute the complete failure-injection matrix in `issues/worker-telemetry-protocol/design.md`: adapter/pi/endpoint crashes, open/update response loss, competing incarnations, stale attempts, retry/terminal races, long tools, settlement/shutdown, storms, malformed/corrupt state, clock jumps, capability path attacks, and stripped-PATH launch. Assert no telemetry case emits reports/status/retry/merge/cleanup effects or deletes work. Benchmark subprocess and lock load, document rollout/rollback, verify autonomous pi and explicit-interactive Claude, and require human approval before enabling the migration gate.
