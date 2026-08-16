---
created: 2026-06-27
updated: 2026-06-28
type: improvement
closed: 2026-06-28
status: done
priority: normal
epic: orchestratectl-mvp
related: ['@core-path-traversal-id-validation', '@supervisor-pid-symlink-containment', '@run-state-symlink-toctou-openat2']
---

# octl-core: symlink/TOCTOU containment for run state dirs

## Description

Spin-off from core-path-traversal-id-validation /llm-review (gpt-5.5, opus).

Id validation prevents traversal via id components, but RunPaths still follows filesystem symlinks: if an attacker can replace <run>/nodes (or discussions/spinoffs, or the run dir itself) with a symlink to /elsewhere, writes land outside the run dir. This is a different threat class (attacker-controlled filesystem vs attacker-supplied id) and was explicitly out of scope for the parent issue.

Decide the threat model and, if in scope, add containment: reject symlinked run subdirs via symlink_metadata before read/write, or O_NOFOLLOW / openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS) on Linux. At minimum document the assumption that the state root is a trusted, per-user 0700 directory.

## Resolution

Added best-effort symlink containment via `symlink_metadata` rejection (pure stdlib, portable). Covers:

- The run root — rejected at construction in both `RunPaths::new` and `RunPaths::from_validated` (the production CLI path), and re-guarded at every access since a handle can be swapped under after construction.
- The `nodes/`/`discussions/`/`spinoffs/` subdirs and every projection file, plus `manifest.json`, `events.jsonl` (append + idempotency read), and the `.lock` file (in `RunLock::acquire`).
- New errors `SymlinkRunDir`, `SymlinkSubdir { name }`, `SymlinkStateFile { name }`; CLI maps all three to the `corrupt_run` envelope (exit 1) carrying the offending path.

Trust model documented in code: state root is `$HOME/.orchestratectl/`, per-user 0700, no shared writers — so symlinks at/above the run root (`runs/`, `$HOME`) are out of scope. The check-then-open TOCTOU residual gap is documented on `reject_symlink`.

Multi-model /llm-review assessment: `assessment.md`. Spin-offs filed: `supervisor-pid-symlink-containment` (extend to the CLI-owned PID file), `run-state-symlink-toctou-openat2` (close TOCTOU with openat2/O_NOFOLLOW + Windows reparse points if the threat model widens).
