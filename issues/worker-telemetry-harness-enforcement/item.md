---
created: 2026-08-22
updated: 2026-08-22
type: task
status: untriaged
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:harness
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
---

# Enforce harness telemetry capability for autonomous runs

## Description

Implement the approved harness capability enforcement and diagnostics slice from `issues/worker-telemetry-protocol/design.md`.

Add versioned adapter probe negotiation with closed stdin, bounded output and timeout; validate a pinned trusted package root/version/integrity and canonical extension entry; explicitly launch pi with ambient extension discovery disabled; issue the attempt capability path; revalidate on retry; and reject autonomous harnesses without a real compatible adapter, including Claude. Interactive use remains honest and optional. Add read-only doctor diagnostics and a human-reviewed rollout/migration gate. Do not auto-install packages. This candidate remains untriaged until human design approval.
