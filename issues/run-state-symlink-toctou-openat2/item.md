---
created: 2026-06-28
updated: 2026-06-28
type: improvement
reporter: jari
status: done
priority: normal
epic: taskfleet-mvp
related: ['@run-state-symlink-containment']
closed: 2026-06-28
---

# taskfleet-core: close symlink TOCTOU with openat2 / O_NOFOLLOW (threat-model widening)

## Description

Spin-off / documented follow-up from run-state-symlink-containment /llm-review (all four reviewers). The parent shipped best-effort symlink containment via `symlink_metadata` rejection. It carries a deliberate, documented residual gap: the check is check-then-open, and the per-level guards (root → subdir → file) share one window — a pure TOCTOU attacker can swap an already-checked component for a symlink before the subsequent open, and a later level then resolves through it. The parent accepts this because the MVP trust model is a per-user `$HOME/.taskfleet/` 0700 state root with no shared writers.

If the threat model ever widens (shared/multi-user mount, untrusted concurrent writers), close the gap properly:

- Linux: `openat2(2)` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS` (or `O_NOFOLLOW` per-component with `*at` family) so the kernel refuses symlink traversal atomically at open time.
- macOS: `O_NOFOLLOW` on `open(2)` closes the final-component case; full subtree containment needs per-component `openat` walking from a dir fd.
- Both require non-stdlib platform crates (e.g. `rustix`/`nix`); the parent kept to pure stdlib for portability.

Also fold in the cross-platform note: the current `FileType::is_symlink()` check catches Unix symlinks but not Windows junctions/reparse points. taskfleet currently targets darwin/linux only, so Windows is not a present concern — but if Windows support is added, reject reparse points (`std::os::windows::fs::FileTypeExt::is_symlink_dir`/`is_symlink_file`, or broad reparse-point rejection) rather than silently following them.

## Resolution (symlink-pack, 2026-06-28)

Done with the **macOS+Linux portable path**; the Linux-only `openat2` path is **deferred** to a future Linux-specific hardening pass.

What shipped (`@symlink-pack`):

- `taskfleet_core::nofollow(&mut OpenOptions)` applies `O_NOFOLLOW` via `OpenOptionsExt::custom_flags(libc::O_NOFOLLOW)` on Unix (macOS + Linux + BSD), no-op elsewhere. Wired into every run-state open that touches the leaf: `events.jsonl` append + torn-tail rewrite, the `.lock` open, the atomic temp-file create, and the projection reads (`read_json`/`read_json_opt`). Projection *writes* already go via temp-file + rename and never open the leaf.
- `supervisor.pid` (CLI-owned, `crates/taskfleet-cli/src/supervise/pid_file.rs`): `symlink_metadata` reject before write (`pid_file_symlink_rejected`), `create_new` + `O_NOFOLLOW` temp write, and `O_NOFOLLOW` reads (a symlinked pid file reads as `None`, never followed). Closes `@supervisor-pid-symlink-containment`.

What this closes vs. leaves open:

- **File level (closed):** `O_NOFOLLOW` atomically refuses a leaf swapped for a symlink *after* the `symlink_metadata` check but *before* the open — the half of the TOCTOU window that `reject_symlink` alone could not close.
- **Directory level (still per-level check-then-open):** a swapped *intermediate* component is covered only by the per-level `symlink_metadata` checks. Fully closing this needs Linux-only `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` walking from a dir fd.

**Deferral rationale:** `openat2` is Linux-only and would split the portable open path into per-platform branches (macOS lacks it; full subtree containment there needs per-component `openat` from a dir fd). Not enough payoff vs. portability complexity under the MVP per-user-`0700` trust model. Revisit only if the threat model widens (shared/multi-user mount, untrusted concurrent writers). The Windows reparse-point note above is also deferred (darwin/linux only today).
</content>
