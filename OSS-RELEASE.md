---
schema_version: 1
status: approved
maturity: mvp
ecosystems: [rust]
targets:
  - {ecosystem: rust, package: taskfleet-core, registry: crates.io,   adapter: cargo-publish-ci}
  - {ecosystem: rust, package: taskfleet,      registry: crates.io,   adapter: cargo-publish-ci}
  - {ecosystem: rust, package: orchestratectl, registry: crates.io,   adapter: cargo-publish-ci}
  - {ecosystem: rust, package: taskfleet,      registry: gh-releases, adapter: cargo-dist}
  - {ecosystem: rust, package: taskfleet,      registry: homebrew,    adapter: cargo-dist}
versioning: semver
changelog: {mode: curated, source: issuectl-trailers}
release: {model: gated, layout: single, bump_hook: "./scripts/shipshape-bump-hook.sh"}
distribution:
  adapter: cargo-dist
  gh_releases: true
  installers: [shell, homebrew]
  homebrew_tap: jarimustonen/homebrew-orchestratectl
  platforms: [aarch64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu]
provenance_level: keyless
dependency_bot: dependabot
health_badges: [ci, registry, license]
license: MIT
docs_site: none
---

> **APPROVED; CUT BLOCKED ON R7.** The exact R6 crates.io saga and Shipshape
> protocol are ready, and credential-free package/plan checks are allowed. The
> workspace intentionally remains at 0.5.1 and `release/taskfleet-release.json`
> remains `activation: blocked-r7` until cargo-dist/Homebrew preparation is
> complete. Do not cut, tag, publish, install, rename the repository, or mutate a
> tap. The old GitHub repository and tap remain truthful until R9/R11.

## Rationale
- **maturity: mvp** — inferred by `ossctl facts`: has CI + a SemVer tag (`v0.0.2-alpha`) rules
  out `spike`; single committer and no ≥1.0 release rules out `production`. README self-labels
  "v0.1.0 pre-release", consistent with mvp.
- **ecosystems: [rust]** — a three-package Cargo workspace: `taskfleet-core`,
  canonical `taskfleet`, and the implementation-free bounded `orchestratectl`
  compatibility wrapper.
- **targets: five declared release legs** — R6 will publish crates in dependency
  order (`taskfleet-core` → `taskfleet` → `orchestratectl`) with exact pins and
  registry visibility/reconciliation between steps. The same tag independently
  triggers Taskfleet GitHub Release and Homebrew legs through cargo-dist. No
  cross-workflow chronology is implied.
- **release.layout: single** — the workspace shares one version (`workspace.package.version =
  0.5.1` in the blocked staging posture); all three packages version and tag together, and cargo-dist treats them as one application
  (`taskfleet`; the Cargo-only `orchestratectl` wrapper is excluded from binary distribution). Not `monorepo` (which implies
  per-package versions/tags).
- **release.model: gated** — cargo-dist's `release.yml` is triggered by a pushed git tag; outside the
  pre-cut block the maintained release wrapper may cut it autonomously after its exact-SHA green gate. Never `auto` (and `auto` is a floor violation on spike, though this is mvp).
- **versioning: semver** — SemVer-style tags already in use (`v0.0.2-alpha`), workspace staged at
  `0.5.1`. Pre-1.0 but the maintainer versions with SemVer, not date-based, so not `calver`.
- **changelog: curated / issuectl-trailers** — a hand-maintained `CHANGELOG.md` already exists
  (cargo-dist reads it to build GitHub Release notes), and single-contributor → `curated`. The
  `issues/` tree + issuectl workflow means changelog content is sourced from issuectl trailers
  (`issuectl changelog`), so `source: issuectl-trailers`. No fragment dir today, matching curated.
- **provenance_level: keyless** — CI-published via cargo-dist, which supports keyless
  attestation; `slsa-l3` is production-only (floor) and this is mvp.
- **dependency_bot: dependabot** — mvp-tier default; none configured yet (`.github/dependabot.yml`
  absent), so this is a proposal for the maintainer to enable.
- **health_badges: [ci, registry, license]** — README already renders CI + License badges; a
  crates.io `registry` badge is warranted once published. `coverage`/`scorecard` are
  production-tier and excluded at mvp.
- **license: MIT** — declared explicitly in `[workspace.package] license = "MIT"` and inherited
  by all workspace packages. Respected as the maintainer's stated choice (not overridden to the Rust
  `MIT OR Apache-2.0` dual-license convention).
- **docs_site: none** — no docs-site generator detected; a docs site is a production-tier concern.

## Release notes
- **The pinned Shipshape 0.10.1 protocol owns the release transaction.** `scripts/shipshape-release.sh plan
  major|minor|patch` seals a non-mutating plan. The plan's bump phase updates
  `[workspace.package].version`, rewrites the exact `taskfleet-core` and
  wrapper-to-`taskfleet` pins, refreshes `Cargo.lock`, finalizes `CHANGELOG.md`,
  runs the declared hook, and commits the result. All three crates.io targets are
  `cargo-publish-ci`; the GitHub Release and Homebrew targets are `cargo-dist`.
  Consequently the host never runs `cargo publish`: the one version tag delegates
  all five publish legs to CI, and the engine observes their results at verify.
- **`release.bump_hook` deterministically regenerates and reviews version fixtures.**
  `./scripts/shipshape-bump-hook.sh` runs the locked `envelope_snapshots` test with
  `INSTA_UPDATE=always`, rejects pending `.snap.new` files and unrelated snapshot
  edits, and runs `scripts/check-version-snapshots.sh`. Its changes are folded into
  Shipshape's bump commit. The hook never installs or mutates a global binary or skill.
- **The exact-SHA pre-tag gate is implemented as a resumable checkpoint.** The
  pinned Shipshape 0.10.1 protocol creates the bump commit inside its clean checkout and otherwise proceeds
  directly to tag push, so the wrapper temporarily rejects only that push. The
  resulting journalled failure leaves the local tag on the exact bump commit. The
  wrapper fast-forwards and pushes `main`, waits for `ci.yml` filtered by that exact
  SHA and `event=push`, then invokes `release resume` only after `gh run watch
  --exit-status` succeeds. Resume pushes the already-created tag and CI owns publish.
  A red or missing main run leaves the release untagged remotely and resumable only
  through `scripts/shipshape-release.sh resume <run-id>`. The wrapper admits only
  Shipshape 0.10.1 commit `3e46568d6969701c5fea82fb134b62aa17121cbe` from the retained
  `jarimustonen/ossctl` source repository. Re-pinning requires a passing manual
  `scripts/test-shipshape-release-0.10-protocol.sh` run against the candidate build,
  recorded in the issue that changes the pin. It reads the bump
  level from the engine's sealed, content-addressed plan and supplies the now-required
  matching `release cut --bump` input; Shipshape still verifies the seal. Any other
  version/build or journal near-miss fails closed and requires revalidation.
- **crates.io publishes are permanent.** Publishing `taskfleet-core@<v>`,
  `taskfleet@<v>`, and the bounded `orchestratectl@<v>` wrapper cannot be
  undone: a version can be yanked but never reused or overwritten. Never publish
  locally. `scripts/publish-crates.sh` packages each exact source archive, then
  reconciles checksum, the complete owner set, internal dependency requirement,
  license/rust-version/repository/homepage/description metadata, and the archive's
  `.cargo_vcs_info.json` commit. Cargo output, including “already exists”, is
  never success evidence. Every dependent starts only after its prerequisite has
  produced a matching index-visible receipt.
- **CI-green tag gate.** From clean, synchronized `main`, seal and inspect the JSON plan,
  then pass its id back to the wrapper:
  ```bash
  scripts/shipshape-release.sh plan patch > /tmp/release-plan.json
  jq . /tmp/release-plan.json
  scripts/shipshape-release.sh cut "$(jq -r .data.plan_id /tmp/release-plan.json)"
  ```
  During `cut`, the wrapper verifies that the local tag points at the journalled bump commit,
  fast-forwards `main` to that commit, pushes `main`, and filters `gh run list` by workflow,
  branch, **exact SHA**, and `event=push`. `gh run watch "$id" --exit-status && shipshape release
  resume …` is load-bearing: only a green run can resume the held tag push. Do not replace the
  wrapper with a direct `shipshape release cut`, bare `shipshape release resume` while the tag is local,
  or `git push <tag>`; each bypasses this project's pre-tag gate. `publish-crates.yml` repeats the full gate for crates.io, while cargo-dist's
  independent `release.yml` makes the pre-tag main check necessary for binaries and Homebrew.
- **Partial success and resume are normal saga states.** The three crates.io jobs,
  GitHub Release, and Homebrew publication remain separately visible. Re-run only
  the failed GitHub workflow/jobs from the same immutable tag. A completed crate
  leg is skipped only after full receipt reconciliation; a mismatch fails closed.
  If a permanent artifact exists but cannot match the source receipt, abandon the
  version and fix forward with a patch—never retag, overwrite, or infer success
  from an error string. `scripts/shipshape-release.sh verify <run-id>` remains the
  read-only cross-leg view, and an interrupted held local tag resumes only through
  the wrapper. If cargo-dist uploaded the GitHub Release but its Homebrew job
  failed because the generated formula produced an empty commit, compare the live
  formula byte-for-byte with the generated `.rb` artifact. If identical, record
  that existing tap commit as the Homebrew receipt and resume verification. If it
  differs, apply the exact generated formula as a normal reviewed commit to the
  configured tap, push that commit, record its SHA/asset checksum, and resume
  verification. Never add `--allow-empty`, delete/recreate the release, or weaken
  the exact-tag/source gates. R7 must adapt this repair recipe to the final tap.
- **Two distribution channels, one tag.** Pushing `vX.Y.Z` triggers both channels. (1)
  **crates.io source publish** through `.github/workflows/publish-crates.yml`, which tests on
  Linux and macOS, checks formatting, clippy, MSRV, docs, cargo-deny, and version snapshots,
  then publishes `taskfleet-core`, `taskfleet`, and `orchestratectl` in dependency order. (2) **Prebuilt binaries + Homebrew
  formula** via **cargo-dist**, configured in `dist-workspace.toml` (`installers = ["shell",
  "homebrew"]`, `publish-jobs = ["homebrew"]`, `hosting = "github"`; aarch64 mac + linux
  targets, mac on a self-hosted runner). `.github/workflows/release.yml` runs `dist`
  for the same tag. It does not publish to crates.io.
- **cargo-dist owns `release.yml`.** It is dist-autogenerated — regenerate via `dist init` /
  `dist generate` after changing the dist config, don't hand-edit, or the next `dist` run clobbers
  your edits.
- **A pushed release tag is cached** — GitHub Releases and Homebrew installer URLs are tied to their
  immutable version tag; deleting a published release leaves dangling installer links.
