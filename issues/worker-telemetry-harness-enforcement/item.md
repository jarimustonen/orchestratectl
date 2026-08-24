---
created: 2026-08-22
updated: 2026-08-24
type: task
status: done
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:harness
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 40
blocked_by: ['@worker-telemetry-cli-surfaces', '@worker-profile-config-resolver']
collision: [run-create, config-harness-selection, run-show-dto, worker-launch]
closed: 2026-08-24
closed_by: pi
---

# Integrate selected agent launch and telemetry visibility

## Description

Integrate the resolver's recorded candidate with process launch and honest telemetry visibility. This slice does not re-resolve candidates or enforce a runtime permission boundary.

## Scope

- Launch the exact recorded user-owned argv without reloading config, advancing fallback, or changing the selected candidate.
- For a selected adapter-capable pi process, export exact full `OCTL_RUN_ID`, `OCTL_NODE_ID`, and absolute `OCTL_ATTEMPT` values.
- Add `requirement` (`required` for autonomous, `optional` for explicit-interactive) and `support` (`configured` or `unsupported`) to `run show` from the recorded interaction and selected candidate, never from sample arrival.
- Preserve existing plain launch, adapter, and worker failure disclosure.
- Prove no alternate create or retry path bypasses the resolver's recorded selection.

## Acceptance Criteria

- [x] Integrated tests cover autonomous pi+adapter, rejected autonomous unsupported pi/Claude, explicit-interactive Claude, local-profile use, launch failure, and retry pinning without duplicating the resolver's selection loop.
- [x] The three `OCTL_*` variables carry exact current identity and are documented as the public launcher contract consumed by the external adapter.
- [x] Interaction remains explicit and is never inferred from run kind, profile name, harness name, or telemetry state.
- [x] No trusted package root, integrity/version attestation, probe negotiation, ambient-extension suppression, launch attestation, capability path/secret, permission enforcement, auto-install, or global harness-setting mutation is introduced.

## References

- `issues/add-configurable-agent/design.md` §§5–7.
- `issues/worker-telemetry-protocol/design.md` §§4–6.

## Resolution

### 2026-08-24T12:57:26Z · @pi

Implemented recorded-candidate launch pinning and telemetry policy visibility; covered create, unsupported/interactive/local profiles, launch failure, exact identity, and retry pinning. Four-model review findings were assessed and all confirmed fixes applied; no residual met the filing bar. Full Rust green gate passed.
