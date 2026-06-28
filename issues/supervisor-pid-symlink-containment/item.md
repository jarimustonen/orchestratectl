---
created: 2026-06-28
updated: 2026-06-28
type: improvement
reporter: jari
status: done
priority: normal
epic: orchestratectl-mvp
related: ['@run-state-symlink-containment']
closed: 2026-06-28
---

# octl-cli: extend symlink containment to supervisor.pid

## Description

Spin-off from run-state-symlink-containment /llm-review (gpt-5.5, opus, deepseek). The parent issue added best-effort symlink rejection for the run dir, the `nodes/`/`discussions/`/`spinoffs/` subdirs, the projection files, `manifest.json`, `events.jsonl`, and `.lock`. `supervisor.pid` was left unguarded because it is a CLI-owned file (`crates/octl-cli/src/supervise/pid_file.rs`) touched at several call sites — `pid_file::write_pid`/`read_pid`, `doctor/checks/data.rs`, `doctor/fix.rs`, `run/reattach.rs`, `supervise/mod.rs` — and is lower-leverage than the event log (it stores a PID; worst case a symlink redirects a small write/read).

Decide whether to guard it, and how to expose the containment primitive to the CLI cleanly:

- `octl_core::paths::reject_symlink` is `pub(crate)`; the CLI can't call it. Options: a public `RunPaths::checked_supervisor_pid() -> Result<PathBuf>` in core (guards root + the pid file, reusing the `SymlinkStateFile { name: "supervisor_pid" }` variant), or a public free-function guard the CLI calls before its own `write_pid`/`read_pid`.
- Route every `supervisor.pid` open through the checked path: PID write (atomic temp+rename — note `pid_file::write_pid` should use `create_new`/O_EXCL for the temp, mirroring `octl_core::atomic`), PID read, the doctor orphan-supervisor scan, and the reattach liveness gate.
- Add tests: write/read reject a symlinked `supervisor.pid`; reattach refuses a symlinked pid file rather than following it.

Trust model is unchanged (per-user 0700 state root, no shared writers); this is best-effort containment with the same check-then-open TOCTOU residual gap documented on the parent.
</content>
