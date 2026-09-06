---
created: 2026-08-22
updated: 2026-08-24
type: task
status: done
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: taskfleet:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:adapter
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
lane: worker-telemetry-adapter
lane_seq: 10
blocked_by: ['@worker-telemetry-cli-surfaces']
collision: [external-pi-adapter-package]
closed: 2026-08-24
commits:
- hash: 79b9cbe
  summary: Publish worker telemetry adapter contract and conformance fixtures
---

# Define external pi telemetry adapter contract

## Description

Define and validate the contract for a small external pi extension/package that depends only on taskfleet's stable public telemetry update endpoint. No adapter runtime or pi extension production code belongs in this repository.

## Repository scope

- Publish the stable public adapter contract: request/response DTO, four-state precedence, 30/90-second refresh/freshness bounds, tool metadata grammar and bounds, 4 KiB cap, two-second send floor, single-flight rule, exact `TASKFLEET_RUN_ID`/`TASKFLEET_NODE_ID`/`TASKFLEET_ATTEMPT` environment names, and privacy exclusions.
- Provide bounded conformance fixtures against the real public endpoint, including valid/invalid payloads, old attempts, endpoint failure, refresh, and event-storm-shaped update sequences.
- Keep the fixtures harness-neutral so the separately owned package can consume them.

## External package obligations

- Use only documented public pi lifecycle events to translate activity into `agent_active`, `tool_running`, `settled`, or `shutdown` with the approved precedence.
- Keep bounded in-memory tool pairing, send only sanitized tool name/count metadata, coalesce event storms with at most one send in flight, and refresh unchanged state every 30 seconds.
- Call only the public telemetry command with supplied identity; bound send frequency and shutdown flush so telemetry cannot block the agent turn indefinitely.
- Test duplicate/unmatched events, endpoint failure, event storms, long tools, refresh, privacy filtering, and shutdown in the owning external package.

## Acceptance criteria

- The repository contract and conformance fixtures pass against the implemented public endpoint.
- This repository contains no adapter runtime, pi event-handling implementation, taskfleet internal import, pi private manager/EventBus access, process-manager integration, session JSONL access, or private-log access.
- Payloads exclude tool arguments, commands, paths, output, prompts, errors, provider/model/session identity, and call IDs.
- Endpoint failure leaves the prior sample to become stale and never becomes run truth.
- No probe executable, package provenance/integrity requirement, trusted root, open/reopen fencing, immutable client sequence, launch attestation, or permission-aware integration is added.

## References

- `issues/worker-telemetry-protocol/design.md` §§2–6 and §8.
- `issues/worker-control-plane-review/integration-review.md` — external-package ownership boundary.

## Resolution

### 2026-08-24T11:09:56Z · @issuectl

Published the typed v1 wire contract and harness-neutral virtual trace fixtures, validated representative requests against the real public endpoint, and kept pi runtime/event handling in the separately owned package. Full Rust gate and multi-model review passed.
