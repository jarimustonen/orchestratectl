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
blocked_by: ['@taskfleet-source-repository-rename']
---

# Cut and verify Taskfleet 0.6.0

## Goal

Complete ADR 0002 R10 end-to-end: harden and activate the canonical Taskfleet release topology after R9, run the mandatory full post-R9 exact-SHA gate, then cut and independently verify Taskfleet 0.6.0 through the repository's pinned Shipshape wrapper.

## Preconditions

- R8 immutable evidence authorizes R9 only; R9 is closed after canonical rename and exact-main CI `33815467669` for `5df8359d092bcb10c26441e988617895151a12a7`.
- GitHub source is repository ID `R_kgDOS3Iezw`, canonical `jarimustonen/taskfleet`; local origin uses the canonical SSH URL.
- Existing public tags/releases stop at v0.5.1; canonical tap head remains prepared empty receipt `db12bb163e47617f0b941a35d3896b6ba0548892`; old tap remains `85ce830378f38cf17283efddd966d5754354e403`.
- Release/distribution activation remains blocked and no v0.6.0 tag/package/release/formula exists.

## Phase A — harden and activate without publishing

- [x] Phase A completed: topology hardened, credentials proven, ledgers activated, and review assessed.

- Re-evaluate and close the R9 review residuals before any live activation: cargo-dist 0.28.2's generated `secrets: inherit` on the reusable gate and the blocked-tag cancellation race/permissive skipped-build host dependency.
- Prefer generator-supported tag-only topology (`pr-run-mode = "skip"` or exact supported equivalent) that removes same-repository PR access to inherited release secrets. Do not hand-edit generated `release.yml`; regenerate with exact cargo-dist 0.28.2 and make generation/check deterministic.
- Establish a structural fail-closed activation boundary for unauthorized/non-wrapper tags. A cancellation timing race is not sufficient. Preserve the held-tag wrapper as the only release path and keep crates/GitHub Release/Homebrew legs independent after the approved tag push.
- Verify repository secret NAMES and required access without exposing values. Any write canary against the canonical tap must be reversible, exact-head CAS guarded, leave the tap at its preflight head, and be fully recorded. Do not mutate the old tap in R10.
- Move release/distribution/tap-secret activation ledgers to the exact ready state only after the hardening and credential gate passes.
- Preserve the exact three-crate publish order (`taskfleet-core` → `taskfleet` → `orchestratectl`) and single Taskfleet cargo-dist app/Homebrew identity.
- Run adversarial review and assess findings before activation is accepted.

## Phase B — mandatory post-R9 exact-SHA gate

- [x] Phase B worker-owned candidate gate completed for `23f7fcf6d9de969300dce560538ce1f3a11f2a2a`; final merged-main push CI remains pending.

- On one exact clean integrated candidate after Phase A, run the complete CI-equivalent Rust gate, docs, reviewed snapshots, release protocol/version/topology fixtures, stripped-PATH tests, three-crate package checks, dual-name/state migration compatibility, disposable archive/shell/Homebrew installs, Shipshape contract/audit/non-mutating 0.6.0 plan, residual/source identity scans, and public availability checks.
- Push candidate through canonical renamed-repository CI including a self-hosted macOS proof. Record exact SHA/tree/run/job/artifact IDs and immutable sanitized evidence.
- No tag, publish, GitHub Release, formula activation, global install, skill install, real state migration, old-tap mutation, or dependent-repository edit occurs in Phase A/B.
- Merge activation/evidence only through Taskfleet. The final merged exact-main SHA must receive green push CI before the release cut.

## Phase C — conductor-owned release transaction

- [ ] Phase C remains conductor-owned and was not executed by this worker.

- From synchronized clean canonical `main`, use only `scripts/shipshape-release.sh plan minor` to seal 0.6.0 and `scripts/shipshape-release.sh cut <plan-id>` to execute. The project autonomy policy removes a human approval pause, but all correctness gates remain mandatory.
- The wrapper must own bump/version pins/lockfile/changelog, held local tag, exact bump-SHA main push CI, and resumed remote tag push. Never run `cargo publish`, bare `shipshape release cut/resume`, manual tag push, retag, or version reuse.
- If pre-tag CI fails, abandon the run and fix forward; remove only the unpushed local tag. If the tag was pushed, publishing may be underway: resume/verify the same journal, never start another.
- Reconcile independently completing crates.io and cargo-dist workflows. Verify all three crates at exact 0.6.0 pins, canonical GitHub Release assets/checksums/attestations/legacy installer stub, embedded commit, fresh archive/shell/Cargo installs, and canonical formula in `jarimustonen/homebrew-taskfleet`.
- Record exact release run/plan/tag/commit/workflow/job/package/asset/formula receipts. Close only after Shipshape verification and every public destination agrees.

## Acceptance Criteria

- [ ] Release topology is structurally hardened and activation ledgers are ready without exposing repository secrets to PR-controlled reusable workflow code.
- [ ] Full post-R9 integrated candidate and final merged main CI are green on recorded exact SHAs.
- [ ] Pinned wrapper cuts exactly v0.6.0; no forbidden direct/manual release action occurs.
- [ ] `taskfleet-core`, `taskfleet`, and bounded `orchestratectl` wrapper 0.6.0 are verified on crates.io with exact pins.
- [ ] Canonical GitHub Release and Taskfleet-only artifacts/installers/checksums are verified.
- [ ] Canonical Homebrew formula is live and fresh canonical installation works.
- [ ] Old tap remains untouched for R11; no dependent repository or real user state is migrated.
- [ ] Immutable evidence is committed and R11 is the only newly authorized migration step.

## Recovery

Fail closed before tag push. After tag push, preserve canonical identity and reconcile the existing release journal/workflows; never roll back published coordinates or retag.
