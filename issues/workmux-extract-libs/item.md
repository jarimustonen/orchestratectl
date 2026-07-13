---
created: 2026-07-13
updated: 2026-07-13
type: improvement
status: open
priority: normal
related: ['@orchestratectl-headless-spawn', '@spinoff-e2e-harness', '@bundle-worktree-merge']
---

# workmux-extract-libs: ask raine to split workmux src/ into lib crates orchestratectl can depend on

_Source: workmux (raine/workmux)_

## Description

Upstream request to raine (`raine/workmux`): split workmux's single-package `src/`
into lib crates so orchestratectl can depend on the multiplexer/git logic directly
in Rust, instead of shelling out to the `workmux` CLI and forwarding flags.

This issue captures a concrete analysis of workmux's current module structure and
the crate split it enables. It supersedes the three inline "if/when raine accepts
the split" references (see **Related**) and the rejected MVP alternative A5.4
(`issues/orchestratectl-mvp/alternatives.md`), which dismissed library embedding on
the grounds that workmux's `lib.rs` surface isn't designed for it — this analysis
argues the module boundaries are already clean enough that a split is realistic.

## Background — workmux is already de-facto modular

The repo is a single `[package]` (not a workspace), **but** `src/` is split into tidy
modules, several of which are already de-facto separate components with clear
interfaces:

| Module | Contents | Usefulness for our use |
|---|---|---|
| `src/multiplexer/` | tmux + kitty + wezterm + zellij + `handle.rs` + `types.rs` (trait-based abstraction) | ☆ **Directly usable** — the multiplexer trait + its tmux impl is exactly what we need for orchestratectl's tmux-cleanup side |
| `src/git/` | branch + worktree + merge + remote + status (~50KB) | ☆ **Directly usable** — could replace our `create.sh` + plain git-shelling |
| `src/sandbox/` | sandbox logic | Useful for some worktree kinds |
| `src/config.rs` | 182KB(!) single-file conf system | Too big, repo-specific |
| `src/command/` | clap verbs (add, remove, merge, etc.) | CLI-only, not usable |

## Proposed crate split

```
workmux-multiplexer  (lib crate — tmux/kitty/wezterm/zellij abstraction)
workmux-git          (lib crate — git worktree wrapper)
workmux-sandbox      (lib crate — sandbox primitives)
workmux-core         (lib crate — shared types, naming)
workmux              (bin crate — depends on the above + command/)
```

## What this enables for us

Dependency in `Cargo.toml`: `workmux-multiplexer = "0.2"` → orchestratectl's supervisor
can call `multiplexer::Tmux::kill_window(window_id)` directly, no shell. Same for the
git side. Headless-spawn becomes one `multiplexer::Tmux::new_session(headless = true)`.

This removes the whole flag-forwarding layer (`--parent-session`, window-name scraping,
`create.sh` stdout-contract parsing) and lets the supervisor's tmux-cleanup path — the
`find_window_by_path` / `git worktree remove` dance — become typed library calls.

## Status of the upstream ask

As of 2026-07-13 there is **no evidence this was ever formally filed to raine** (no
GitHub issue/PR link found in orchestratectl or homebase). This issue is currently a
captured internal plan; the next step is to open it upstream at `raine/workmux` (or
confirm an existing discussion) before we can depend on any of it.

Installed workmux is 0.1.211; latest on the `raine/homebrew-workmux` tap is 0.1.220
(2026-07-12) — no crate split has shipped in that range.

## Out of scope

- `config.rs` (too big, repo-specific) and `command/` (CLI-only) stay in the bin crate.
- We do not block on this: flag-forwarding via `create.sh` is sufficient today. This is
  a cleanliness/coupling improvement, not a functional gap.
