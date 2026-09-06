---
created: 2026-08-22
updated: 2026-08-24
type: task
status: done
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: taskfleet:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:e2e
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 50
blocked_by: ['@worker-telemetry-core-control', '@worker-telemetry-cli-surfaces', '@worker-profile-config-resolver', '@worker-telemetry-harness-enforcement', '@worker-telemetry-pi-adapter']
collision: [worker-control-plane-integration]
closed: 2026-08-24
closed_by: pi
commits:
- hash: ccdedc9
  summary: 'test: validate worker control plane rollout'
---

# Validate and roll out worker telemetry end to end

## Description

Validate and document the approved simplified worker control plane after the taskfleet slices and adapter contract/conformance work are available.

## Scope

- Exercise autonomous configured pi+adapter, explicit-interactive Claude, and user-owned local-profile flows in isolated test configuration.
- Validate real-adapter flows through an ordinarily installed external package when available; prove taskfleet-side endpoint and state-integrity invariants independently with an in-repo fake driver that uses only the public contract.
- Cover current/stale/missing/corrupt samples, old attempts, malformed payloads, long tools, settlement/shutdown, event storms, clock jumps, endpoint/adapter/worker failure, launch failure, retry races, and stripped ambient `PATH`.
- Assert requested/selected/fallback visibility and profile residency/telemetry preservation across create, dry-run, stored state, retry, and `run show`.
- Run a modest subprocess and lock-load check, and document installation, rollout, failure disclosure, and rollback across the repository/external-package boundary.

## Acceptance Criteria

- [x] Every telemetry case proves no sample alone emits a report, changes status, selects retry, satisfies `run wait`, proves merge/landing, classifies an outcome, authorizes cleanup, or deletes work.
- [x] Autonomous unsupported candidates fail clearly; Claude succeeds only when explicit-interactive; the existing local `secure` profile is usable without special restrictions.
- [x] When a real external adapter is available, its validation installs only in an isolated test environment; meanwhile the repository fake contains no pi production adapter code and calls only the stable public endpoint. The unavailable package delivery is transferred to `@uncommonly-vague-family`.
- [x] No security-boundary, permission/operation-set, capability-path, package-integrity, probe, launch-attestation, sequence/epoch, or elaborate provenance tests are retained.

## References

- `issues/worker-telemetry-protocol/design.md` §§7–8.
- `issues/add-configurable-agent/design.md` §§7–9.
- `issues/worker-control-plane-review/integration-review.md` — approved end-to-end flows and prerequisites.

## Resolution

### 2026-08-24T13:30:26Z · @pi

Taskfleet-owned rollout scope is complete. Evidence is indexed in `docs/WORKER-CONTROL-PLANE-ROLLOUT.md`: profile selection covers autonomous adapted pi, explicit-interactive Claude, local residency, deterministic fallback, dry-run/create/stored state/show, exact argv/identity, launch failure, and retry pinning; endpoint/core suites cover strict DTOs, freshness/corruption/old attempts, clock behavior, races, and modest subprocess/lock load. Every published endpoint case and accepted reference send now inventories all non-telemetry run files, constrains telemetry to exactly one advisory sample, preserves the full simulated worktree, and verifies `run show`/`run wait` remain pending, unreported, and unlanded.

The production external pi adapter package does not exist in this repository or the available installed package set. It was not installed or validated. Harness hooks, coalescing timers, and bounded shutdown remain external-package obligations transferred to unlaned follow-up `@uncommonly-vague-family`; repository reference traces are explicitly not presented as adapter-runtime execution. This issue closes only the taskfleet-side acceptance requested by the owning run.
