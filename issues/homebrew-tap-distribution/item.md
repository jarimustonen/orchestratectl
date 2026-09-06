---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: done
priority: normal
closed: 2026-08-04
---

# Distribute taskfleet via a Homebrew tap (match issuectl/ossctl)

## Description

## Problem

`taskfleet` is the odd one out among Jari's three cross-machine CLIs. `issuectl` and `ossctl` ship via Homebrew taps (`jarimustonen/issuectl`, `jarimustonen/ossctl`) and are kept current on every machine by a simple `brew upgrade` in a `dotfiles/setup.d/<tool>.sh` hook (run ~hourly by homebase's login auto-sync). `taskfleet` instead ships cargo-from-source: its `dotfiles/setup.d/taskfleet.sh` hook clones the GitHub repo and runs `cargo install --path crates/taskfleet-cli`.

Consequences of the cargo-from-source path:
- Requires **git + SSH auth + a full Rust toolchain** on every machine just to install/update.
- **Slow cold build** on a fresh machine (whole cargo workspace); every source-changing pull triggers a recompile.
- Diverges from the uniform "all three CLIs handled the same way" model Jari wants — issuectl/ossctl are one `brew upgrade`, taskfleet is a bespoke clone+build.
- No prebuilt bottle: nothing to `brew install` without the source tree.

## Goal

Distribute `taskfleet` via a Homebrew tap exactly like issuectl/ossctl, so the homebase setup hook collapses to the same three-line `brew list/upgrade/install` shape and no Rust toolchain is needed on consumer machines.

## Work

1. **Release pipeline** in this repo: tag-triggered release that builds the CLI binary and publishes artifacts (mirror whatever issuectl/ossctl do — check their repos for the pattern: GitHub release + bottle upload + formula bump).
2. **Tap repo**: create `jarimustonen/homebrew-taskfleet` with the `taskfleet` formula (analogue of `homebrew-issuectl` / `homebrew-ossctl`).
3. Verify `brew install jarimustonen/taskfleet/taskfleet` works on a clean Mac and that the binary still ships + installs its companion skills (`taskfleet skill install --force`).

## Homebase-side follow-up (do NOT do here — tracked separately, trivial once the tap exists)

In `~/Sources/homebase`:
- Add `jarimustonen/taskfleet` to `dotfiles/setup.d/brew-trust.sh` TAPS.
- Add `jarimustonen/taskfleet/taskfleet` to `dotfiles/src/brew-packages.txt`.
- Rewrite `dotfiles/setup.d/taskfleet.sh` to the brew-upgrade shape used by `issuectl.sh` / `ossctl.sh` (keeping the global `taskfleet skill install --force` step, since taskfleet's skills are machine-global — unlike issuectl's repo-local `/issue` skill).

## Comments

- Keep the global skill-install step: ossctl's hook is the closest model (brew upgrade + `skill install --force` + leftover-prune + lockstep check).
- `issuectl`'s skill is intentionally repo-local and stays that way — this migration is about distribution parity, not skill parity.

## Resolution (done)

Tap published by Jari: `jarimustonen/tap/taskfleet` (formula in `jarimustonen/homebrew-tap`), stable 0.1.0.

Homebase side landed (commits in `~/Sources/homebase`):
- `dotfiles/setup.d/brew-trust.sh` — trust `jarimustonen/tap`
- `dotfiles/src/brew-packages.txt` — add `jarimustonen/tap/taskfleet`
- `dotfiles/setup.d/taskfleet.sh` — rewritten from clone+`cargo install` to the brew-upgrade shape; also retires the legacy `~/.cargo/bin/taskfleet` (it sat before `/opt/homebrew/bin` on PATH and would otherwise shadow the brew binary) and prunes dangling worktree-skill symlinks before `skill install --force`.

Verified idempotent on gertrud + hauis; haapa (no brew) skips cleanly; brunhild migrates on next auto-sync. All three cross-machine CLIs (issuectl / ossctl / taskfleet) now provision identically via `brew upgrade` in their `setup.d` hooks.

### Upstream follow-up worth noting
`taskfleet skill install --force` aborts the entire install when it hits a pre-existing **symlink** at a target path (error `refused_overwrite`, even with `--force`). `--force` arguably should replace a symlink (at least a dangling one). Worked around in the homebase hook by pruning broken symlinks first, but consider making `--force` overwrite symlinks upstream.
