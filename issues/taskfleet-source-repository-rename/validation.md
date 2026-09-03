# ADR 0002 R9 source-repository rename validation

## Authority and boundary

R8 evidence commit `488d6cab7fc8ca883f7c660a695097441cf9c407`
was an ancestor of the R9 source branch. Its fail-closed verifier passed over 69
artifacts and reported `r9_authorized=true`, `release_authorized=false`. The R8
production commit was `c3ef8b740ac531f12ce81c759ed209d178cf36bd`
(tree `b7d07d9df3308fb33afdfab892f949f46ef810d4`) and exact-SHA CI run
`33764612111` was successful.

Immediately before mutation, the authenticated source repository was public
`jarimustonen/orchestratectl`, default branch `main`, immutable repository ID
`1265770191` and node ID `R_kgDOS3Iezw`; `jarimustonen/taskfleet` returned 404.
Current main CI run `33810002421` had completed successfully. Shipshape reported
zero in-flight/unreadable release runs, both release workflows had zero active
runs, and the tag, release and tap snapshots matched the R8/R7 receipts.

This evidence authorizes only R9. No release, tag, publication, GitHub Release,
tap/formula activation, global installation, skill installation, state
migration, dependent-repository edit, or local checkout rename was performed.

## One-way repository mutation

The authenticated owner renamed the existing repository to
`jarimustonen/taskfleet`. The canonical API immediately returned the same ID
`1265770191` and node ID `R_kgDOS3Iezw`, with public visibility and default
branch `main`. The shared Git origin fetch and push URLs were immediately set to
`git@github.com:jarimustonen/taskfleet.git`; all active worktrees consume that
shared config. The old name was not recreated. Its web URL now emits a GitHub
301 to the canonical URL, observed only as compatibility evidence; no maintained
operation in this validation uses that redirect.

## Maintained source convergence

Candidate commit `076f983c498de1ca2fc8fe0b919130ffbd52dc27`
(tree `06aaf232a85833ac1762e7a2fcf89b38cf9e6572`) changes:

- Cargo repository/homepage metadata, README badge/source location, discussions
  URL and maintained changelog links to `jarimustonen/taskfleet`;
- release topology, release-wrapper fixtures, registry reconciler user-agent and
  metadata fixtures to canonical source identity;
- cargo-dist from prepared dispatch-only mode to its generated version-tag
  trigger while preserving `blocked-r8-r9-r10` release activation,
  `prepared-blocked-r10` distribution state and the inert tap secret;
- generated cargo-dist output using exact 0.28.2, with canonical plan hosting at
  `/jarimustonen/taskfleet/releases/...` and the same canonical Homebrew tap;
- the generated activation-gate caller's missing `contents: read` permission;
- CI with a same-repository-PR-only self-hosted macOS ARM64 acceptance job,
  while hosted macOS remains available for ordinary and fork PR coverage; and
- active R7→R9 instructions so R10, not R9, owns live tap credentials and release
  activation.

The residual classifier finds zero unclassified occurrences and zero maintained
exact old source URLs. Retained old coordinates are classified as the legacy
Homebrew tap/formula, accepted decision text, frozen 0.5.1/legacy-skill fixtures,
or issue/evidence history. The bounded `orchestratectl` Cargo wrapper, state and
environment compatibility, all `OCTL_*` variables, telemetry contract ID,
old-tap migration artifacts and historical evidence remain unchanged.

## Local verification

The complete post-review local gate passed:

- `cargo fmt --all --check`;
- `cargo test -p taskfleet`, followed by a zero-`.snap.new` check;
- `cargo clippy --locked --workspace --all-targets -- -D warnings`;
- `cargo nextest run --locked --release --workspace`;
- `cargo test --locked --release --workspace --doc`;
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`;
- release-wrapper, held-tag and crates.io reconciliation scripts;
- cargo-dist 0.28.2 `generate --check` and `plan`;
- distribution topology and residual identity validators; and
- `git diff --check`.

The reviewed insta loop produced no changed snapshots. The complete sanitized log
and cargo-dist plan are indexed evidence.

## Canonical operations and candidate CI

A fresh no-checkout SSH clone from `git@github.com:jarimustonen/taskfleet.git`
and an explicit fetch of the candidate branch both resolved the exact candidate
SHA. The candidate branch push was made directly to canonical origin and checked
through the canonical authenticated API.

Temporary PR [#1](https://github.com/jarimustonen/taskfleet/pull/1) targeted
`main` but was never merged through GitHub. Exact candidate runs:

- CI `33814447787`: successful at the exact candidate SHA. All nine jobs passed:
  rustfmt, version snapshots, clippy, hosted Ubuntu tests, hosted macOS tests,
  self-hosted macOS ARM64 tests, MSRV 1.85, docs and cargo-deny.
- Release `33814447929`: successful non-publishing PR plan. The plan and reusable
  activation gate passed; every build, host, release and Homebrew publish job was
  skipped.

The self-hosted job `test (self-hosted-macos-arm64)` completed successfully on
repository runner ID 21 with labels `self-hosted`, `macOS`, `ARM64`. Runner name
is intentionally sanitized. After the gates, PR #1 was closed unmerged and its
remote candidate branch was deleted; the local worker branch remained intact for
`taskfleet run merge`.

Canonical authenticated API, SSH clone, fetch, reversible candidate push, PR and
Actions operations therefore work without redirect dependence.

## Review and residual release risk

Four reviewers completed an independent pass, one bounded context follow-up and
two cross-review rounds. Four confirmed source/plan findings were fixed before
candidate CI: gate checkout permission, self-hosted fork exposure, unavailable
job-level matrix context and stale R9 activation/token prose. Exact cargo-dist
0.28.2 source disproved the claim that GitHub `HostStyle::Create` mutates GitHub
before the gate.

The review retained one accepted pre-R10 risk: cargo-dist 0.28.2's generated host
job accepts skipped build dependencies and therefore relies on the activation
gate's early whole-run cancellation in a blocked tag run. The generated reusable
call also emits `secrets: inherit`. This is the already documented R7 workaround,
no tag exists, activation remains blocked and the Homebrew secret remains inert.
R10 must re-evaluate both before installing live credentials or setting
activation to `ready`. It is not release authorization.

## No-release/no-tap proof

Before/after tag-ref count is 28 and digest is
`16ac4238a89bf6108ec7564dc054ef3daa723185a805bdcfb3590753b5673e4a`.
The repository still has 17 historical GitHub Releases. Shipshape has zero
in-flight and zero unreadable release runs. Release workflows have zero active
runs after candidate completion.

The old tap remains commit `85ce830378f38cf17283efddd966d5754354e403`
(tree `f52b102239003614edf65eadcd34931b44a9cc0d`), and the canonical tap remains
its empty proof commit `db12bb163e47617f0b941a35d3896b6ba0548892`
(tree `4b825dc642cb6eb9a060e54bf8d69288fbee4904`). Actions secret names and update
timestamps are unchanged; values were never read or persisted.

## Conductor-owned finalization

This worker must not close the issue. After `taskfleet run merge`, the conductor
must:

1. verify canonical `origin/main` equals the exact merged SHA;
2. wait for a fresh `ci.yml` **push** run at that exact SHA, with every job green,
   including `test (self-hosted-macos-arm64)` on runner ID 21;
3. recheck repository ID/name, canonical remotes, residual classifier,
   tag/release/tap and secret-name invariants;
4. record the merged commit and exact-main CI on the issue; and only then
5. close `taskfleet-source-repository-rename`.

Until those steps pass, R9 remains open and R10/release remain blocked.
