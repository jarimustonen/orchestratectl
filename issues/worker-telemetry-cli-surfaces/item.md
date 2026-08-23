---
created: 2026-08-22
updated: 2026-08-22
type: task
status: untriaged
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:endpoint
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
---

# Implement worker telemetry CLI and read surfaces

## Description

Implement the approved public `orchestratectl node telemetry open|update` CLI/JSON contract and presentation surfaces from `issues/worker-telemetry-protocol/design.md`.

Cover strict versioned DTOs, capability-file transport metadata, flags/JSON normalization, state-field validity, input/output limits, idempotent responses, stable error/action classes, bounded lock acquisition, absolute update-rate enforcement, and process/write/lock benchmarks. Expose per-node detail in `run show`, count-only aggregation in `run list`, and no telemetry in `run wait` v1. Telemetry must remain excluded from generic attention, health, progress, terminal, retry, merge, and cleanup modules. This candidate remains untriaged until human design approval.
