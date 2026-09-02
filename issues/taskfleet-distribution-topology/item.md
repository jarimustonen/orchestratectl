---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: done
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
closed: 2026-09-02
commits:
- hash: 028005a
  summary: distribution-topology
- hash: 007dd40
  summary: protocol-fixture
---

# Prepare Taskfleet cargo-dist and Homebrew topology

## Description

## Goal

Prepare, but do not activate, the canonical `homebrew-taskfleet` repository/token proof and the old tap's atomic migration commit. Configure cargo-dist 0.28.2 for Taskfleet app/assets, one canonical formula and a non-installing old latest-installer stub; regenerate `release.yml`. Prepare exact repository URL/action/secret/runner/release-wrapper substitutions. New Homebrew/shell/archive channels must not ship an `orchestratectl` alias.

**Acceptance:** the only allowed public mutations are creation of the empty canonical `homebrew-taskfleet` repository and one reversible token-proof commit; record receipts and leave the old tap untouched. cargo-dist PR plan machine-checks exactly one distributed app (`taskfleet`), canonical archives/checksums/installer/formula plus one non-installing old installer stub, and zero old wrapper binaries/formulae/assets; only one generated tap target; disposable Homebrew plans reviewed. No old-tap activation, canonical publication, GitHub source-repository rename, release tag or install.

## Acceptance Criteria

- [x] cargo-dist 0.28.2 plans exactly one Taskfleet app, one canonical tap,
  canonical assets, and only the bounded non-installing old-name stub.
- [x] Canonical tap creation/token-write proof is receipted; the live tap remains
  empty and the broad proof credential has been replaced with an inert secret.
- [x] Old-tap migration is prepared against exact identity without mutating the tap.
- [x] Disposable Homebrew and real native artifact drills pass without installation.
- [x] R8-R10 activation gates remain blocked and no release/publication occurred.
- [x] Multi-model review findings, full Rust gates, packaging, and pinned Shipshape
  protocol validation pass.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `80`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-skills-docs-contracts`, `@taskfleet-release-machinery`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.

## Resolution

### 2026-09-02T20:11:15Z · @issuectl

R7 complete: canonical cargo-dist/Homebrew topology is prepared and machine-verified; release activation remains blocked on R8, R9, and R10. Validation and receipts are in this issue directory.
