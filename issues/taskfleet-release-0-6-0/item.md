---
created: 2026-09-04
updated: 2026-09-06
type: task
reporter: jari
status: open
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 110
collision: [repository-identity]
---

# Cut and verify Taskfleet 0.6.0

## Goal

Complete ADR 0002 R10 truthfully. The first canonical coordinate, v0.6.0, was wrapper-authorized and then burned when both independent tag workflows failed closed before publication. R10 therefore includes the successful v0.6.1 fix-forward release and verifies that every intended public channel agrees.

## Preconditions and pre-cut work

- [x] R9 closed after the canonical source rename; repository ID `R_kgDOS3Iezw` is `jarimustonen/taskfleet`.
- [x] Release topology was hardened to tag-only cargo-dist operation, protected exact-commit authorization refs, independent crates/distribution gates, exact three-crate order, and the canonical Taskfleet-only Homebrew tap.
- [x] Phase A activation and credential proofs passed without exposing secret values or mutating the old tap.
- [x] Phase B full post-R9 candidate gate passed at `23f7fcf6d9de969300dce560538ce1f3a11f2a2a`; its exact same-repository CI was `34014432088`.

## Release transaction outcome

### Burned v0.6.0 coordinate

- [x] The pinned wrapper advanced v0.6.0 commit `57f6dfb83401694399b363de5d3aa88e4541a22c` through exact-main CI `34016341659` and created its protected exact-commit authorization ref before pushing the tag.
- [x] Crates workflow `34016740702` and cargo-dist workflow `34016740704` failed closed in their authorization gates. No v0.6.0 crate, GitHub Release, release asset, or canonical formula was published.
- [x] Shipshape journal `01M1TNW3SMN0XA347D1MG4518R` is terminally abandoned at sequence 38. The v0.6.0 tag and authorization ref remain immutable at the release commit and must never be deleted, moved, or reused.

### Successful v0.6.1 fix-forward

- [x] The release-gate portability defects were fixed and verified before a new patch coordinate was sealed.
- [x] v0.6.1 tag and authorization ref resolve to `7e93bd6195fbaf6de0b43d9161228ae2373ab5d1`; exact-main CI `34020144153` passed.
- [x] Crates workflow `34020495272` published `taskfleet-core`, `taskfleet`, and the bounded `orchestratectl` wrapper in dependency order; cargo-dist workflow `34020495260` published the GitHub Release, assets, and canonical formula.
- [x] Shipshape journal `01M1TTRXNXK6FPQJK3F92B9AXA` completed at sequence 51 with all five targets `matches`. Its post-public direct resume occurred only after 5/5 read-only reconciliation and could neither bypass the pre-tag gate nor republish.

## Acceptance Criteria

- [x] Release topology is structurally hardened and exact-main CI is green on both immutable release commits.
- [x] v0.6.0 is recorded as a burned, unpublished coordinate: only its immutable tag and authorization ref exist.
- [x] v0.6.1 is available as non-yanked `taskfleet-core`, `taskfleet`, and bounded `orchestratectl` crates with registry checksums, source commit, and exact dependency pins verified.
- [x] The v0.6.1 GitHub Release has the exact target commit and complete digest-verified Taskfleet-only asset set; archive runtime identity, shell installer, and inert legacy installer stub pass in disposable homes.
- [x] `jarimustonen/homebrew-taskfleet` contains the v0.6.1 canonical formula, and a fresh install/uninstall in a fully disposable non-temporary prefix proves only `taskfleet` is installed, with the expected version/commit and no residue.
- [x] The old tap remains at `85ce830378f38cf17283efddd966d5754354e403`; no dependent repository, installed user binary/skill, or real user state was migrated.
- [x] The checksummed final evidence bundle under `evidence/final/` verifies, and the completed R10 authorizes ADR 0002 R11 only.

## Recovery and next authorization

Published identities are never rolled back or reused. v0.6.0 remains a permanent burned tag receipt; v0.6.1 is the first fully published canonical Taskfleet release. R11 may now activate and verify only the prepared old-tap Homebrew migration. Dependent-repository discovery or migration remains outside this authorization and starts only after R11's own gate.
