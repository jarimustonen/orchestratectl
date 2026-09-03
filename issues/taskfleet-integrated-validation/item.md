---
created: 2026-09-02
updated: 2026-09-03
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
blocked_by: ['@taskfleet-distribution-topology', '@publish-crates-fixture-symlink-chmod', '@taskfleet-native-materialization', '@native-spawn-test-leaks', '@pre-r8-ci-portability']
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

## Agent Runs

### 2026-09-02T20:24:08Z · @taskfleet-r8-worker

Integrated validation pinned exact source commit `fa04841ad74c0ea935cc8c81a83a90a917678853`, then stopped at a mandatory gate failure.

Authoritative main CI run `33678068490` for that exact SHA completed **failure**. Job `100407635030` (`version-snapshots`) failed in `./scripts/test-publish-crates.sh`: the fixture symlinks host prerequisites into `$tmp/bin` and then runs `chmod +x "$tmp/bin/"*`; on Linux this dereferences the links and attempts to chmod `/usr/bin/{awk,bash,jq,...}`, which is refused with `Operation not permitted`. Both Ubuntu and macOS Rust test jobs passed, as did fmt, clippy, docs, cargo-deny, MSRV, snapshots, bump-hook, and release-wrapper fixture legs, but exact-SHA CI is red and the registry protocol fixture did not execute.

Fresh read-only checks before stopping found: crates.io `taskfleet` and `taskfleet-core` endpoints 404 (facts, not reservations); the current source repository remains `jarimustonen/orchestratectl`; `jarimustonen/taskfleet` is 404; the canonical tap exists at empty-tree proof commit `db12bb163e47617f0b941a35d3896b6ba0548892`; the old tap still contains formula blob `c7d02e0e61f16e347f01bed09473fa7b86b5034f`; Homebrew core formula API endpoints for both names are 404.

Filed unlaned intake bug `@publish-crates-fixture-symlink-chmod` with exact run provenance. Per R8 acceptance, this failure blocks closure and does **not** authorize R9. The local full gate and pinned Shipshape build were cancelled after the authoritative CI blocker became conclusive; stripped-PATH, parity/adoption/migration, disposable packaging/Homebrew, sealed 0.6.0 plan, evidence review, and `/assess-findings` therefore remain intentionally incomplete and must be rerun from scratch against the corrected exact integrated commit. No repository/tap mutation, publish, tag, install, state/skill migration, or source rename occurred.
