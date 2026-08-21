---
created: 2026-08-21
updated: 2026-08-21
type: task
reporter: jari
status: done
priority: high
lane: skills
collision: [crates/octl-cli/skills/stint-start/SKILL.template.md]
closed: 2026-08-21
commits:
- hash: 1596dd2
  summary: distinguish untriaged work from explicit deferral
- hash: fe6ea2e
  summary: distinguish untriaged work from explicit deferral after rebase
---

# Distinguish untriaged work from explicit deferral

## Description

The bundled stint-start instruction currently says: “Leave deferred or out-of-plan entries unscheduled.” This conflates two distinct states and allowed migrated review residuals to become `status: deferred` while retaining lane metadata. issuectl treats deferred as active-class, so a deferred lane head was then reported spawnable and serialized the lifecycle lane.

Maintainer clarification, 2026-08-20:

- An unaccepted review residual, out-of-plan finding, or candidate that has not passed the human lane-or-close gate remains `status: untriaged` and has no lane/lane_seq/collision scheduling assignment.
- `status: deferred` is reserved for an explicit human/product disposition of an accepted worthwhile item (“not now”), and it also remains unscheduled.
- An agent must not turn “not selected this round” into a deferred disposition; simply leave it untriaged/out of plan.

The three migrated residuals `shell-quote-dedup`, `run-merge-stamp`, and `enforce-run-merge` were corrected directly to untriaged with lane/lane_seq removed.

## Acceptance Criteria

- [x] Bundled stint guidance states the distinction above unambiguously.
- [x] Work selection never treats an unscheduled row's mechanical `spawnable:true` as executable; it must first pass triage and gain a lane.
- [x] Deferred is described as an explicit human/product disposition, never an agent-created parking state.
- [x] Relevant stint templates/mirrors and deterministic skill checks/snapshots are updated consistently.
- [x] The exact repository green gate passes.

## Resolution

### 2026-08-21T13:45:58Z · @issuectl

Bundled stint guidance now keeps unaccepted candidates untriaged and unscheduled, reserves deferred for explicit human/product disposition, rejects mechanical spawnability without an executable lane/status, and pins Claude/pi/Codex output semantics. Full repository green gate passed under a stripped PATH.
