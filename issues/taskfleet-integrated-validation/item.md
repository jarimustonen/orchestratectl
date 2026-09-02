---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: open
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R8
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 90
blocked_by: ['@taskfleet-distribution-topology']
collision: [repository-identity]
---

# Produce immutable integrated Taskfleet pre-cut evidence

## Description

## Goal

On one exact integrated commit run the full Rust/clean-PATH/docs/snapshot/issue gates; both-name command parity; 0.5.1 terminal/active/pending/unknown/config/provenance adoption; optional migration/refusal/rollback cases; disposable Cargo/archive/shell/Homebrew flows; Shipshape contract/audit/plan; and fresh crates/GitHub/tap checks. Record immutable command outputs, hashes and commit identity on the issue.

**Acceptance:** every ADR pre-cut leg passes on the same commit and a committed evidence index records command manifest, toolchain, output hashes, CI/artifact identifiers and exact SHA; side-effecting commands are stubbed/credential-isolated and every mutation destination is sandboxed. Any failure blocks repository rename. R8 evidence authorizes R9 only and expires when R9 changes repository identity; R10 requires a full post-R9 exact-SHA integrated rerun on its actual candidate. R8 performs no additional GitHub/tap mutation, publish, tag, global install or real-state migration; local Homebrew simulations are labelled pre-live, while R10/R11 own hosted formula and cross-tap proof.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `90`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-distribution-topology`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
