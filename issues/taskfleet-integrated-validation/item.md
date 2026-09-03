---
created: 2026-09-02
updated: 2026-09-04
type: task
reporter: taskfleet-r0-worker
status: done
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
closed: 2026-09-04
closed_by: taskfleet-r8-worker
---

# Produce immutable integrated Taskfleet pre-cut evidence

## Description

## Goal

On one exact integrated commit run the full Rust/clean-PATH/docs/snapshot/issue gates; both-name command parity; 0.5.1 terminal/active/pending/unknown/config/provenance adoption; optional migration/refusal/rollback cases; disposable Cargo/archive/shell/Homebrew flows; Shipshape contract/audit/plan; and fresh crates/GitHub/tap checks. Record immutable command outputs, hashes and commit identity on the issue.

**Acceptance:** every ADR pre-cut leg passes on the same commit and a committed evidence index records command manifest, toolchain, output hashes, CI/artifact identifiers and exact SHA; side-effecting commands are stubbed/credential-isolated and every mutation destination is sandboxed. Any failure blocks repository rename. R8 evidence authorizes R9 only and expires when R9 changes repository identity; R10 requires a full post-R9 exact-SHA integrated rerun on its actual candidate. R8 performs no additional GitHub/tap mutation, publish, tag, global install or real-state migration; local Homebrew simulations are labelled pre-live, while R10/R11 own hosted formula and cross-tap proof.

## Acceptance Criteria

- [x] Every required ADR 0002 R8 pre-cut leg passed against exact commit `c3ef8b740ac531f12ce81c759ed209d178cf36bd`.
- [x] Immutable command, toolchain, CI, artifact, review, sanitization, and residue evidence is committed under this issue.
- [x] Every authoritative side-effecting execution used isolated/stubbed destinations; exploratory incidents and warnings are explicitly dispositioned.
- [x] Authority is restricted to R9 and explicitly excludes release, publish, tag, install, tap activation, and R10/R11 actions.

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

### 2026-09-03T21:42:27Z · @taskfleet-r8-worker

Integrated pre-cut validation completed against exact source commit `c3ef8b740ac531f12ce81c759ed209d178cf36bd` (tree `b7d07d9df3308fb33afdfab892f949f46ef810d4`). All required R8 legs passed, including exact-SHA CI, 1,115 release tests, docs/clippy/fmt, state/config/provenance adoption, 33-surface dual-name parity, three-crate packaging, canonical and bounded-legacy Cargo installs, archive/shell channels, real old-receipt Homebrew migration/upgrade/uninstall/fresh install, Shipshape 0.10.1 contract/audit/sealed plan, public identity facts, review, and residue.

The stripped-PATH run passed all tests with disclosed nextest delayed-exit and xcrun SDK lookup warnings; the ordinary gate was clean. Failed exploratory setup attempts are preserved separately and do not count as gate evidence. Four early probes touched the real legacy dispatch log before isolation was fixed; exact lines were removed, but no pre-digest exists and mtimes changed. Task-owner disposition and final clean authoritative replacements are recorded.

Four-model `/llm-review` plus two cross-review rounds and `/assess-findings` produced no surviving product defect. Confirmed evidence gaps were fixed and rerun. The remote model-performance corpus append failed closed because haapa was out of space; local model assessment is committed.

The immutable evidence under this issue authorizes only R9's source-repository rename. It does not authorize release, tag, publication, installation, tap activation, or R10/R11 work, and expires when R9 changes repository identity.
