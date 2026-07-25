---
created: 2026-07-25
updated: 2026-07-25
type: improvement
status: open
priority: normal
related: ['@supervisor-spawn-fails-silently-at-run-create']
---

# run create supervisor confirmation should use a readiness pipe, not a pid-file poll timeout

## Description

The fail-loud supervisor confirmation (`run create` → `spawn_for_run`) still polls
`<run-dir>/supervisor.pid` for a bounded time (`PID_FILE_WAIT`, now 15s) to decide
whether the detached supervisor booted. This is inherently ambiguous: a supervisor
that boots slowly under heavy load (fork storms, cgroup CPU throttling, degraded I/O)
can exceed the deadline while it is perfectly healthy — `run create` then returns
`supervisor_spawn_failed`, but the grandchild keeps running and will `claim_pid_atomic`,
write its pid, and supervise a run the caller was told failed (an orphan).

Raised by all four reviewers in the creation-path reliability review (see
`history/review-creation-path-reliability.md`, finding F4b). The 15s timeout only
lowers the probability; it does not remove the ambiguity.

**Sound fix:** replace the time-bound pid-file poll with a UNIX daemonization
readiness pipe threaded through the double-fork in
`crates/octl-cli/src/run/supervisor_spawn.rs`:
1. Parent creates a pipe; the intermediate inherits the write end.
2. The grandchild writes a readiness byte (or a structured error) AFTER
   `claim_pid_atomic` + runtime init, then closes the write end.
3. The parent blocks on `read()` — EOF-without-byte means the supervisor died
   during init (fate-sharing); a byte means confirmed; an error payload gives the
   real reason.

This eliminates the arbitrary timeout and the orphan window. Touches the async-signal-safe
`pre_exec` path, so it needs its own design + tests (readiness success, init-failure EOF,
partial write).
