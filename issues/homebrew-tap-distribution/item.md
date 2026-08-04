---
created: 2026-08-04
updated: 2026-08-04
type: feature
status: open
priority: normal
---

# Distribute orchestratectl via a Homebrew tap (match issuectl/ossctl)

## Description

## Problem

`orchestratectl` is the odd one out among Jari's three cross-machine CLIs. `issuectl` and `ossctl` ship via Homebrew taps (`jarimustonen/issuectl`, `jarimustonen/ossctl`) and are kept current on every machine by a simple `brew upgrade` in a `dotfiles/setup.d/<tool>.sh` hook (run ~hourly by homebase's login auto-sync). `orchestratectl` instead ships cargo-from-source: its `dotfiles/setup.d/orchestratectl.sh` hook clones the GitHub repo and runs `cargo install --path crates/octl-cli`.

Consequences of the cargo-from-source path:
- Requires **git + SSH auth + a full Rust toolchain** on every machine just to install/update.
- **Slow cold build** on a fresh machine (whole cargo workspace); every source-changing pull triggers a recompile.
- Diverges from the uniform "all three CLIs handled the same way" model Jari wants — issuectl/ossctl are one `brew upgrade`, orchestratectl is a bespoke clone+build.
- No prebuilt bottle: nothing to `brew install` without the source tree.

## Goal

Distribute `orchestratectl` via a Homebrew tap exactly like issuectl/ossctl, so the homebase setup hook collapses to the same three-line `brew list/upgrade/install` shape and no Rust toolchain is needed on consumer machines.

## Work

1. **Release pipeline** in this repo: tag-triggered release that builds the CLI binary and publishes artifacts (mirror whatever issuectl/ossctl do — check their repos for the pattern: GitHub release + bottle upload + formula bump).
2. **Tap repo**: create `jarimustonen/homebrew-orchestratectl` with the `orchestratectl` formula (analogue of `homebrew-issuectl` / `homebrew-ossctl`).
3. Verify `brew install jarimustonen/orchestratectl/orchestratectl` works on a clean Mac and that the binary still ships + installs its companion skills (`orchestratectl skill install --force`).

## Homebase-side follow-up (do NOT do here — tracked separately, trivial once the tap exists)

In `~/Sources/homebase`:
- Add `jarimustonen/orchestratectl` to `dotfiles/setup.d/brew-trust.sh` TAPS.
- Add `jarimustonen/orchestratectl/orchestratectl` to `dotfiles/src/brew-packages.txt`.
- Rewrite `dotfiles/setup.d/orchestratectl.sh` to the brew-upgrade shape used by `issuectl.sh` / `ossctl.sh` (keeping the global `orchestratectl skill install --force` step, since orchestratectl's skills are machine-global — unlike issuectl's repo-local `/issue` skill).

## Notes

- Keep the global skill-install step: ossctl's hook is the closest model (brew upgrade + `skill install --force` + leftover-prune + lockstep check).
- `issuectl`'s skill is intentionally repo-local and stays that way — this migration is about distribution parity, not skill parity.
