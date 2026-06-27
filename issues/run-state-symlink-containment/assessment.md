# Multi-model review assessment — symlink containment

Reviewers: gemini-3.1-pro, gpt-5.5, claude-opus-4-7, deepseek-v4-pro.
Full transcript: `history/` review output (not committed).

## Disposition table

| # | Finding (consensus) | Disposition |
|---|---------------------|-------------|
| 1 | `events.jsonl` append unguarded — the source-of-truth write follows a symlink outside the run dir (all 4) | **FIX** |
| 2 | `from_validated` skips the root check; production CLI uses it, so the construction-time guard was test-only (all 4) | **FIX** — make it `Result`, guard root |
| 3 | `.lock` unguarded — `flock`/open follow a symlinked lock file (all 4) | **FIX** — guard in `RunLock::acquire` |
| 4 | `supervisor.pid` unguarded (gpt, opus, deepseek) | **SPIN OFF** — CLI-side, many call sites, lower leverage (writes a PID) |
| 5 | Multi-step `symlink_metadata` shares one TOCTOU window; ancestor symlinks (`runs/`, `$HOME`) not caught (gemini, opus, deepseek) | **DOC** — ancestors out of scope by the per-user 0700 trust model; full closure needs openat2 → spin-off |
| 6 | `write_json_atomic` tmp file could be a pre-created symlink (gemini) | **REFUTE** — `create_new(true)` = `O_EXCL` fails on an existing symlink; `rename` replaces the link without following it. Add a doc note. |
| 7 | `SymlinkProjectionFile` is semantically wrong for `manifest.json` (gpt, opus) | **FIX** — rename → `SymlinkStateFile { name }`, one variant for every run-state file |
| 8 | `corrupt_run` drops the symlinked path / kind from the JSON envelope (opus) | **FIX** — attach the path via `with_invalid_value` |
| 9 | `&'static str` discriminator is typo-prone; use an enum (opus) | **WONTFIX** — matches the existing `CorruptProjection.kind` convention; out of scope |
| 10 | 3× `symlink_metadata` syscalls per read; cache root/subdir (opus) | **WONTFIX** — premature; correctness over a micro-opt on a non-hot CLI path |
| 11 | A non-symlink replacement (regular file where a dir is expected) maps to `io_error`, not `corrupt_run` (gpt) | **WONTFIX-now** — not a symlink escape; acceptable as generic I/O |
| 12 | Windows junctions/reparse points bypass `is_symlink()` (gpt) | **DOC** unix-only + **SPIN OFF** |
| 13 | Missing tests: events/lock/manifest symlinks, write-side file symlinks, dangling symlinks (all 4) | **FIX** |

## What was kept as solid
- The id-validation vs symlink-rejection separation; `_opt` "missing is fine,
  corrupt is not" semantics preserved on the symlink axis; honest TOCTOU/
  trust-model docs; real-symlink `tempfile` tests for the leaf cases.
</content>
</invoke>
