---
created: 2026-08-22
updated: 2026-08-24
type: task
status: open
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:endpoint
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 20
blocked_by: ['@worker-telemetry-core-control']
collision: [octl-core-schema, run-show-dto, run-list-dto]
---

# Implement worker telemetry CLI and read surfaces

## Description

Expose the approved harness-neutral telemetry update endpoint and compact freshness read views.

## Scope

- Add one strict `orchestratectl node telemetry update` command accepting either payload flags or `--input-file <PATH|->`, never both.
- Reject raw input larger than 4 KiB before normalization or mutation, normalize the versioned request into the core update DTO, and return the standard JSON envelope with receive and expiry timestamps.
- Add per-node telemetry to `run show`: sample state, last-told activity, age, state elapsed time, attempt, and bounded tool metadata.
- Add only bounded sample-state counts to `run list`.
- Use observational text such as “last told” and “run status unchanged.”

## Acceptance criteria

- Flags, strict JSON, stdin/file input, invalid combinations, raw input size, and stable machine-readable errors are tested.
- `run show` hides old-attempt samples and distinguishes absent, current, stale, clock-unreliable, and invalid data.
- `run list` remains bounded and applies the same current-attempt and freshness rules as `run show`; old-attempt samples count as absent.
- `run wait`, attention, health/progress inference, reports, retry, merge, outcomes, and cleanup do not consume telemetry.
- `requirement` and `support` are not emitted by this slice; they are integrated only after a candidate is selected and recorded.
- There is no `open` command, capability-file transport, sequence idempotency protocol, or elaborate error/action taxonomy.

## References

- `issues/worker-telemetry-protocol/design.md` §§4–5 and §7.
- `issues/worker-control-plane-review/integration-review.md` — approved CLI/read-surface reshape.
