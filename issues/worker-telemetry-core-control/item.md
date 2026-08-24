---
created: 2026-08-22
updated: 2026-08-24
type: task
status: open
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:core
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 10
collision: [octl-core-schema, octl-core-reducer]
---

# Implement worker telemetry control and bounded sample

## Description

Implement the orchestratectl-owned telemetry storage boundary from the approved simplified worker-control-plane design.

## Scope

- Validate strict versioned updates for an existing run and node, reject terminal nodes, and require the exact current attempt.
- Under the ordinary run lock, atomically replace one bounded sample at `telemetry/<node-id>.json`; cap the normalized DTO and stored data at 4 KiB.
- Store only run/node/attempt identity, the four-state activity value, sanitary bounded tool metadata, server-maintained `state_since`, `received_at`, and `expires_at`.
- Reset `state_since` when the sample attempt changes even if the activity enum is unchanged.
- Compute 90-second freshness with an injected clock; classify missing, corrupt, old-attempt, and backward-clock data as absent, invalid, or clock-unreliable as specified.
- Keep telemetry outside `events.jsonl`, `applied_seq`, manifest/node progress timestamps, reducer status/report/retry/merge state, typed outcomes, and cleanup decisions.

## Acceptance criteria

- Strict validation rejects unknown fields, invalid enum/field combinations, oversized normalized data, unknown or terminal nodes, and non-current attempts without mutation.
- Atomic replacement, corruption, partial-write, clock, retry/terminal race, current-attempt behavior, and attempt-boundary `state_since` are tested.
- Negative tests prove telemetry alone cannot emit or synthesize a report, status, retry, merge, landing, `run wait` settlement, outcome, or cleanup action.
- No capability secret/file, authorization claim, control projection, open/incarnation handshake, epoch, or client sequence is introduced.

## References

- `issues/worker-telemetry-protocol/design.md` §§2–5 and §7.
- `issues/worker-control-plane-review/integration-review.md` — approved product and state-integrity boundaries.
