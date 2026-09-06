---
created: 2026-08-24
updated: 2026-08-24
type: task
reporter: jari
status: done
priority: normal
lane: policy
lane_seq: 20
closed: 2026-08-24
commits:
- hash: 7f86541
  summary: schedule simplified worker control plane DAG
---

# Reshape worker control plane implementation DAG

## Description

## Goal

Turn the approved, simplified worker-control-plane design into an executable issuectl DAG without implementing production code.

## Binding source

Use `issues/worker-control-plane-review/integration-review.md` post-decision assessment and Jari's recorded decision. The rejected permission, operation-set, capability-secret, package-attestation, probe-negotiation, and elaborate provenance machinery must not survive in candidate issue bodies or acceptance criteria.

## Scope

- Rewrite the five existing `worker-telemetry-*` candidate issue bodies to their smallest approved reshape scopes.
- Accept them from `untriaged` to `open` because Jari approved proceeding through the described implementation steps.
- Create one clear profile configuration/resolver implementation issue for user-owned executable definitions, capability/residency profiles, deterministic non-weakening fallback, and compact requested/selected visibility.
- Define explicit dependencies and conservative lane/collision metadata. Sequence shared state schema, reducer, run-create, config, and DTO hot surfaces; do not rely on optimistic lane disjointness.
- Represent the external pi adapter as an external-package deliverable with a documented repository/package boundary. Do not put pi-specific internals into taskfleet.
- Make end-to-end rollout depend on every implementation prerequisite.
- Close this planning issue when the DAG validates.

## Acceptance Criteria

- [x] Every accepted issue is self-contained, readable, and free of rejected complexity.
- [x] The DAG is acyclic, has no missing blockers, and exposes a safe first executable head.
- [x] Existing five issue slugs are preserved rather than duplicated.
- [x] No production code or adapter implementation is added.
- [x] `issuectl dag --json --reservations '[]'` and `issuectl doctor --json` pass.

## Resolution

The five candidates were reshaped and accepted, `worker-profile-config-resolver` was filed through intake and accepted, and the implementation DAG was validated after multi-model review and findings assessment. Commit `7f86541` records the implementation-plan changes.
