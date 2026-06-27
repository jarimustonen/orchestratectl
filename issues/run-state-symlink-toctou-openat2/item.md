---
created: 2026-06-28
updated: 2026-06-28
type: improvement
reporter: jari
status: open
priority: low
epic: orchestratectl-mvp
related: ['@run-state-symlink-containment']
---

# octl-core: close symlink TOCTOU with openat2 / O_NOFOLLOW (threat-model widening)

## Description

Spin-off / documented follow-up from run-state-symlink-containment /llm-review (all four reviewers). The parent shipped best-effort symlink containment via `symlink_metadata` rejection. It carries a deliberate, documented residual gap: the check is check-then-open, and the per-level guards (root → subdir → file) share one window — a pure TOCTOU attacker can swap an already-checked component for a symlink before the subsequent open, and a later level then resolves through it. The parent accepts this because the MVP trust model is a per-user `$HOME/.orchestratectl/` 0700 state root with no shared writers.

If the threat model ever widens (shared/multi-user mount, untrusted concurrent writers), close the gap properly:

- Linux: `openat2(2)` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS` (or `O_NOFOLLOW` per-component with `*at` family) so the kernel refuses symlink traversal atomically at open time.
- macOS: `O_NOFOLLOW` on `open(2)` closes the final-component case; full subtree containment needs per-component `openat` walking from a dir fd.
- Both require non-stdlib platform crates (e.g. `rustix`/`nix`); the parent kept to pure stdlib for portability.

Also fold in the cross-platform note: the current `FileType::is_symlink()` check catches Unix symlinks but not Windows junctions/reparse points. orchestratectl currently targets darwin/linux only, so Windows is not a present concern — but if Windows support is added, reject reparse points (`std::os::windows::fs::FileTypeExt::is_symlink_dir`/`is_symlink_file`, or broad reparse-point rejection) rather than silently following them.
</content>
