---
created: 2026-08-14
updated: 2026-08-14
type: task
status: done
priority: normal
closed: 2026-08-14
---

# declare release.bump_hook for version-snapshot regen (ossctl --bump readiness for 0.2.0)

## Description

Release-readiness prep for the upcoming orchestratectl 0.2.0 cut. ossctl 0.5.0's
engine-owned `release cut --bump major|minor|patch` bumps `[workspace.package]
version`, rewrites the intra-workspace `=<ver>` pin, refreshes `Cargo.lock`,
finalizes the CHANGELOG, then runs a contract-declared `release.bump_hook` (folding
its edits into the bump commit) and **fails closed** if the hook exits non-zero or
leaves the version altered. The `version` command output is snapshotted in
`crates/octl-cli/tests/snapshots/envelope_snapshots__version_{text,json,jsonl}.snap`,
which bake the literal crate version and go stale on a bump — this exact gap turned
`main` CI red after the v0.1.8 tag. This issue declares + validates the `bump_hook`
that auto-regenerates those snapshots during the cut. NOT the 0.2.0 cut itself.

## Chosen hook command

```
INSTA_UPDATE=always cargo test -p orchestratectl --test envelope_snapshots
```

**Why this form** (over `cargo insta test --accept -p orchestratectl` or a `.snap.new`
find/mv accept loop):
- **Dependency-free.** `cargo-insta` is NOT installed here (`which cargo-insta` → not
  found) and must not be assumed present in a CI/cut environment. `INSTA_UPDATE=always`
  is read by the `insta` crate at test runtime — no external tool.
- **Self-contained, no post-processing.** `INSTA_UPDATE=always` rewrites the mismatched
  `.snap` files **in place** and the test **passes** (exit 0) — no `.snap.new` rename
  loop, no `|| true` masking of genuine failures.
- **Fails closed correctly.** A real compile/test break exits non-zero → the executor
  aborts the cut. A version-only bump exits 0 with exactly the snapshots refreshed.
- **Scoped.** `--test envelope_snapshots` is the only test binary embedding the crate
  version, so a bump rewrites just the three `version_*` snapshots — proven empirically
  (see scratch diff). It never edits `[workspace.package] version`, satisfying the
  executor's post-hook guard.

## Step 3 — scratch-bump proof (0.1.8 → 0.2.0, discarded)

Manually bumped `[workspace.package] version` 0.1.8→0.2.0 + the `octl-core` `=0.1.8`→
`=0.2.0` pin (mirroring the executor's version edit), rebuilt, ran the hook:

- (a) all three `version_*` snapshots now embed `0.2.0` (grep → single token `0.2.0`).
- (b) `scripts/check-version-snapshots.sh` → **pass** (`match workspace version 0.2.0`);
  before the hook it failed on all three (stale `0.1.8`).
- (c) `cargo test --workspace` → **green** (0 failed).
- (d) the hook changed **only** the three snapshots — `git diff --stat` after the hook
  showed exactly `version_{json,jsonl,text}.snap`; the other diff entries (`Cargo.toml`,
  `crates/octl-cli/Cargo.toml`, `Cargo.lock`) are the executor's version edit, not the
  hook's. No `.snap.new` leftovers.

Scratch bump fully reverted (`git checkout`); repo back at 0.1.8, snapshots matching.

## Step 4 — ossctl 0.5.0 contract + plan validation

Binary: `/Users/jari/Sources/ossctl/target/release/ossctl` (0.5.0; PATH `ossctl` is
the older 0.2.2, unused).

- `contract validate --json` → `valid: true`, zero warnings.
- `contract show --json` → `release.bump_hook` parsed verbatim into the contract.
- `release plan --bump minor --json`:
  - (a) surfaces `bump.bump_hook` verbatim + a "review it as trusted code" warning
    echoing the command.
  - (b) derives BOTH crates as dep-ordered crates.io publish units — `targets: [octl-core,
    orchestratectl]` (library first) — even though the contract declares only
    `orchestratectl` as a target.
  - (c) carries the homebrew tap: `homebrew_tap: jarimustonen/homebrew-orchestratectl`.
  - `pin_rewrites: [orchestratectl→octl-core =0.1.8→=0.2.0]`; `phases: [bump, dry-run-all,
    build-all, publish-all, tag, dist]`.

## Contract change discovered during step 4c

The plan's `homebrew_tap` is sourced **only** from the contract's `distribution` block
(`ossctl-core/src/release/plan.rs:177` — `contract.distributions.find_map(|d|
d.homebrew_tap.clone())`), NOT from `dist-workspace.toml`. The `dist` phase is
unconditional (fixed in `PlanPhase::SEQUENCE`), so it appeared even while the tap was
`null`. To make the plan carry the tap (criterion 4c), added a v2 `distribution:` block
to `OSS-RELEASE.md` mirroring `dist-workspace.toml` **exactly** (adapter cargo-dist;
installers [shell, homebrew]; tap `jarimustonen/homebrew-orchestratectl`; platforms
aarch64-darwin + aarch64/x86_64-linux-gnu) — zero drift, correct v2 modeling (the
`distribution` layer coexists with `targets`; it is NOT a crates.io target).

## Green gate

`cargo fmt --all --check` (OK), `cargo clippy --workspace --all-targets -- -D warnings`
(0 warnings), `cargo test --workspace` (all green), `scripts/check-version-snapshots.sh`
(match 0.1.8). Change is docs/config only (`OSS-RELEASE.md`) — Rust untouched.

## Description

