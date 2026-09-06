---
created: 2026-08-24
updated: 2026-08-24
type: task
reporter: jari
status: done
priority: normal
lane: policy
lane_seq: 10
closed: 2026-08-24
---

# Remove stint local source installation

## Description

## Goal

Remove source-tree `cargo install --path ...` from taskfleet's stint/deploy operating policy. Repository work builds and tests repository-local artifacts; it does not replace the user's globally installed taskfleet binary or reinstall bundled instructions as a stint side effect.

## Product decision

Jari decided 2026-08-23 that building this repository does not imply installing it locally. The current root AGENTS.md orchestrator-only local reflection rule is superseded. Workers were already forbidden from global installation; extend the no-global-mutation boundary to the orchestrator's normal stint flow.

## Scope

- Update root AGENTS.md operating policy and duplicated lifecycle/deploy snippets.
- Update the active TODO handoff so it no longer requests commit-equality local deployment.
- Preserve release behavior: releases remain tag/CI driven; no local `cargo publish`.
- Preserve worker-local validation via `cargo build --release` and explicit `./target/release/taskfleet` execution.
- State clearly that installed release binaries are upgraded through the distribution channel, outside repository build/test work.
- Close `considerably-utter-deer` as obsolete after policy documentation lands: the transient post-`cargo install --path` provenance probe no longer occurs in the supported stint workflow.

## Acceptance Criteria

- [x] No normal stint instruction invokes `cargo install --path crates/taskfleet-cli` or mutates global taskfleet installation/installed skills.
- [x] Green-gate and release rules remain intact.
- [x] Documentation distinguishes repository-local build/test from distribution-channel installation.
- [x] The stale provenance intake is closed obsolete with this policy decision recorded.

## Resolution

### 2026-08-24T08:04:08Z · @issuectl

Updated root operating policy, active handoff, contributor guidance, and bundled workflow instructions: repository work has no stint deploy step, validates local artifacts explicitly, and never mutates the installed taskfleet or bundled skills. Release publication remains tag/CI driven.
