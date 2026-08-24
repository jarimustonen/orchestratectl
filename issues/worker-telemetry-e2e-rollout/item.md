---
created: 2026-08-22
updated: 2026-08-24
type: task
status: open
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:e2e
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 50
blocked_by: ['@worker-telemetry-core-control', '@worker-telemetry-cli-surfaces', '@worker-profile-config-resolver', '@worker-telemetry-harness-enforcement', '@worker-telemetry-pi-adapter']
collision: [worker-control-plane-integration]
---

# Validate and roll out worker telemetry end to end

## Description

Validate and document the approved simplified worker control plane after the orchestratectl slices and adapter contract/conformance work are available.

## Scope

- Exercise autonomous configured pi+adapter, explicit-interactive Claude, and user-owned local-profile flows in isolated test configuration.
- Validate real-adapter flows through an ordinarily installed external package when available; prove orchestratectl-side endpoint and state-integrity invariants independently with an in-repo fake driver that uses only the public contract.
- Cover current/stale/missing/corrupt samples, old attempts, malformed payloads, long tools, settlement/shutdown, event storms, clock jumps, endpoint/adapter/worker failure, launch failure, retry races, and stripped ambient `PATH`.
- Assert requested/selected/fallback visibility and profile residency/telemetry preservation across create, dry-run, stored state, retry, and `run show`.
- Run a modest subprocess and lock-load check, and document installation, rollout, failure disclosure, and rollback across the repository/external-package boundary.

## Acceptance criteria

- Every telemetry case proves no sample alone emits a report, changes status, selects retry, satisfies `run wait`, proves merge/landing, classifies an outcome, authorizes cleanup, or deletes work.
- Autonomous unsupported candidates fail clearly; Claude succeeds only when explicit-interactive; the existing local `secure` profile is usable without special restrictions.
- Real external-adapter validation installs only in an isolated test environment; the fake driver contains no pi production adapter code and calls only the stable public endpoint.
- No security-boundary, permission/operation-set, capability-path, package-integrity, probe, launch-attestation, sequence/epoch, or elaborate provenance tests are retained.

## References

- `issues/worker-telemetry-protocol/design.md` §§7–8.
- `issues/add-configurable-agent/design.md` §§7–9.
- `issues/worker-control-plane-review/integration-review.md` — approved end-to-end flows and prerequisites.
