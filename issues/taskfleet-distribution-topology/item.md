---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: open
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R7
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 80
blocked_by: ['@taskfleet-skills-docs-contracts', '@taskfleet-release-machinery']
collision: [repository-identity]
---

# Prepare Taskfleet cargo-dist and Homebrew topology

## Description

## Goal

Prepare, but do not activate, the canonical `homebrew-taskfleet` repository/token proof and the old tap's atomic migration commit. Configure cargo-dist 0.28.2 for Taskfleet app/assets, one canonical formula and a non-installing old latest-installer stub; regenerate `release.yml`. Prepare exact repository URL/action/secret/runner/release-wrapper substitutions. New Homebrew/shell/archive channels must not ship an `orchestratectl` alias.

**Acceptance:** the only allowed public mutations are creation of the empty canonical `homebrew-taskfleet` repository and one reversible token-proof commit; record receipts and leave the old tap untouched. cargo-dist PR plan machine-checks exactly one distributed app (`taskfleet`), canonical archives/checksums/installer/formula plus one non-installing old installer stub, and zero old wrapper binaries/formulae/assets; only one generated tap target; disposable Homebrew plans reviewed. No old-tap activation, canonical publication, GitHub source-repository rename, release tag or install.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `80`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-skills-docs-contracts`, `@taskfleet-release-machinery`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
