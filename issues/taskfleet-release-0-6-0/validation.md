# R10 Phase A/B validation

## Scope and boundary

This worker completed only Phase A and the pre-merge candidate portion of Phase B. It did not create or push a tag, publish a crate, create a GitHub Release, activate a formula, install Taskfleet globally, invoke `skill install`, migrate real state, modify either tap, or execute Phase C. The final merged-main push CI and release transaction remain conductor-owned.

## Exact candidate

- Commit: `23f7fcf6d9de969300dce560538ce1f3a11f2a2a`
- Tree: `fbabcec6898d9529758eb79f5f42182bd866b9e4`
- Canonical source: `jarimustonen/taskfleet` (`R_kgDOS3Iezw`)
- Disposable same-repository PR: [#2](https://github.com/jarimustonen/taskfleet/pull/2), closed unmerged and branch deleted
- CI run: [`34014432088`](https://github.com/jarimustonen/taskfleet/actions/runs/34014432088), green on the exact head SHA

## Phase A result

The generated cargo-dist 0.28.2 workflow is tag-only (`pr-run-mode = "skip"`), contains no PR trigger or inherited reusable-workflow secrets, and invokes the same fail-closed authorization verifier in every local build. The crates workflow invokes that verifier independently. The held-tag wrapper creates a protected, version-scoped authorization ref only after exact-main CI and before resuming Shipshape's held tag.

GitHub rulesets `22234415` (release tags) and `22234417` (authorization refs) are active, target the intended ref patterns, contain no bypass actors, and block creation/update/deletion outside repository-administrator authority. The canonical Homebrew credential was synchronized through Homebase; only sanitized secret metadata and a reversible exact-head canary receipt are recorded here. Both tap heads remained unchanged.

Activation ledgers now agree on `ready`, `active-proven-r10`, tag-push/PR-skip operation, the protected authorization ref, the canonical source, the canonical tap, the exact three-crate order, and Taskfleet-only binary distribution.

## Phase B result

The candidate passed:

- the full Rust green gate without fail-fast, including clippy, all release tests, doctests, and rustdoc warnings-as-errors;
- reviewed version-bearing snapshots and deterministic bump-hook fixtures;
- an isolated Shipshape 0.10.1 held-tag/cut/resume/verify protocol at the pinned upstream commit;
- strict release authorization, wrapper, publish-order, topology, activation, live-ruleset, and issue-health fixtures;
- a stripped-PATH all-workspace nextest run;
- all three `cargo package --workspace --no-verify` archives and disposable Cargo installs for the canonical and compatibility binaries;
- exact cargo-dist 0.28.2 generation/plan validation plus disposable archive, shell-installer, and Homebrew 6.0.21 drills;
- canonical source metadata, exact intra-workspace pins, and the expected single R11-only old-tap README residue;
- Shipshape contract validation, audit, and a sealed non-mutating minor plan to 0.6.0;
- same-repository hosted Linux/macOS and self-hosted ARM64 macOS CI, including the checksum-pinned cargo-dist topology job;
- postflight confirmation that v0.6.0 remains absent from all three crates, tags, and GitHub Releases.

No cargo-dist release workflow ran for the PR; the exact-head workflow list contains only the successful CI run. CI produced no artifacts, which is expected for this validation workflow.

## Review disposition

Two adversarial rounds and a context follow-up were assessed in `evidence/assessment.{json,md}`. Every confirmed in-scope release-safety or evidence gap was fixed. The retained constraints are explicit upstream/trust-boundary facts: cargo-dist 0.28.2 emits workflow-wide `contents: write`, its host tolerates skipped local jobs, and a repository administrator remains the policy authority. No review residual met the bar for a new issue.

## Remaining conductor gate

After Taskfleet merges this branch, the conductor must wait for green push CI on the exact merged `main` SHA before invoking the Phase C wrapper. Phase C remains unchecked and no release coordinate is authorized by this document.
