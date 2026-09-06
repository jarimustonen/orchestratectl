# R10 final validation

## Scope and immutable pre-cut evidence

Phase A hardened the two independent tag workflows and activated the release ledgers. Phase B passed its full post-R9 gate at candidate `23f7fcf6d9de969300dce560538ce1f3a11f2a2a` (tree `fbabcec6898d9529758eb79f5f42182bd866b9e4`, CI `34014432088`). The original checksummed evidence remains in `evidence/`; `evidence/final/` is a separate post-release bundle and does not rewrite those pre-cut receipts.

The topology remained the approved five-target saga: `taskfleet-core` → `taskfleet` → bounded `orchestratectl` on crates.io, plus independent GitHub Release and Homebrew targets for Taskfleet. cargo-dist is tag-only, every publishing leg requires the protected version-specific authorization ref, and the wrapper records that ref only after exact-main CI.

## v0.6.0: correctly authorized, then burned

The pinned wrapper advanced release commit `57f6dfb83401694399b363de5d3aa88e4541a22c` through exact-main CI `34016341659`, then created `refs/heads/taskfleet-release-authorizations/v0.6.0` at that same commit before publishing the annotated `v0.6.0` tag.

Both tag workflows failed closed before publication:

- crates workflow `34016740702`: authorization/topology job `101441707888` failed; all three publication jobs were skipped;
- cargo-dist workflow `34016740704`: local build jobs `101441745244`, `101441745248`, and `101441745351` failed their authorization checks; host, release, and formula jobs were skipped.

The failures were jq 1.6 reserved-variable incompatibility and unavailable privileged ruleset fields under the workflow token. No v0.6.0 crate, GitHub Release, asset, or formula exists. Shipshape journal `01M1TNW3SMN0XA347D1MG4518R` is terminally `abandoned` at sequence/applied sequence 38. The tag and authorization ref remain immutable and are not release-success claims.

## v0.6.1: successful fix-forward

The gate fixes landed before the patch cut. The source fixes needed for truthful Shipshape reconciliation were also verified in the Shipshape source repository (`jarimustonen/ossctl`): tracked-helper observation commits `dc1bdf8`/`bf29567` with exact-main CI `34021860186`, and GNU Homebrew verification commits `a7b2ae1`/`88f2b98` with exact-main CI `34022689350`.

The v0.6.1 tag and protected authorization ref both resolve to `7e93bd6195fbaf6de0b43d9161228ae2373ab5d1`; exact-main CI `34020144153` passed before tag publication. Both independent tag workflows then passed:

- crates workflow `34020495272` published all three packages in dependency order;
- cargo-dist workflow `34020495260` published the GitHub Release, all release assets, and the canonical formula.

Shipshape journal `01M1TTRXNXK6FPQJK3F92B9AXA` is `completed` at sequence/applied sequence 51 with all five configured targets `matches`. Its direct resume occurred after the tag was already public and after 5/5 read-only verification. It could not bypass the pre-tag gate and did not republish.

## Independent public reconciliation

Fresh queries used the declared non-default User-Agent `taskfleet-r10-evidence/1.0 (https://github.com/jarimustonen/taskfleet)` and failed on any disagreement. `evidence/final/public-state.json` records:

- HTTP 404 for every v0.6.0 package version and the v0.6.0 GitHub Release;
- the two-commit canonical tap history going directly from the empty proof commit to the v0.6.1 formula, with no v0.6.0 formula;
- all three v0.6.1 crates non-yanked, downloaded bytes matching registry checksums, `.cargo_vcs_info.json` at the exact release commit, and exact `=0.6.1` dependency pins;
- GitHub Release target, metadata, complete asset names/sizes/API IDs, and downloaded SHA-256 values matching GitHub's asset digests;
- every nonblank `sha256.sum` entry matching its downloaded asset (the cargo-dist blank separator line is formatting only);
- canonical formula version/archive digest and tap head `c9e68594340b2b775d23159a3545d53f15306471`;
- unchanged old-tap head `85ce830378f38cf17283efddd966d5754354e403` and its still-live 0.5.1 formula.

The downloaded macOS archive contained `taskfleet` and no `orchestratectl`; its runtime reported version 0.6.1 and embedded commit `7e93bd6195fbaf6de0b43d9161228ae2373ab5d1`. The legacy installer stub exited 1, pointed to the canonical installer, and did not mutate its disposable home. The canonical shell installer installed only `taskfleet` into a disposable `CARGO_HOME`, with the same runtime identity.

## Fresh disposable Homebrew proof

`evidence/final/verify-homebrew-install.sh` cloned Homebrew into a freshly created non-`/tmp` prefix under the user's cache area, isolated `HOME` and `HOMEBREW_CACHE`, disabled auto-update/analytics/cleanup, bounded every Homebrew command to 300 seconds, and cleaned the root on every exit. It used the public canonical tap, explicitly trusted it non-interactively, and never accessed the user's system taps or Cellar.

The fresh canonical installation passed: Homebrew installed exactly one formula (`taskfleet`), exposed only the `taskfleet` command, and the binary reported 0.6.1 with the exact embedded release commit. After uninstall, the disposable Cellar and formula list were empty and neither command link remained. The receipt is `evidence/final/homebrew-install-result.txt`. Earlier `/tmp` refusal and stalled cache-prefix attempts are diagnostics only and are not represented as passes.

## Boundary and authorization

No source/release-workflow change, tag, publication, tap mutation, global install, skill install, dependent-repository edit, or real user-state migration occurred in this final-evidence worker. The old tap remains deliberately unchanged for R11.

The checksummed `evidence/final/index.json` covers every final receipt and its collector/verifier. `evidence/final/verify-evidence.sh`, `issuectl doctor --json`, and `git diff --check` passed. This closes R10 through the successful v0.6.1 fix-forward and authorizes only ADR 0002 R11 (old-tap Homebrew migration), not post-live dependent-repository convergence.
