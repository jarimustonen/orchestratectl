---
created: 2026-07-25
updated: 2026-07-25
type: improvement
status: fixed
priority: normal
related: ['@supervisor-spawn-fails-silently-at-run-create']
closed: 2026-07-25
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

## Comments

### 2026-07-25T12:49:25Z · @claude

Landed: readiness pipe replaces the pid-file poll in run create's confirmation path (spawn_for_run). The grandchild writes R<pid>/E<code> after claim_pid_atomic+init and closes; the parent poll()s the read end. Slow-but-healthy boots are confirmed with no deadline to overrun; a dead one is caught by fate-sharing EOF. No orphan window.

Reviewed by a 4-model /llm-review panel (history/review-readiness-pipe-raw.md, triage in history/assessment-readiness-pipe.md). Applied the confirmed hardening (2nd commit): a generous 120s wedge backstop (a purely unbounded read would hang forever behind the BLOCKING claim_pid_atomic flock — verified in octl-core lock.rs), CLOEXEC-both-ends + clear-in-pre_exec (the CLI runs a tracing_appender worker thread, so the write-end leak race is real), strict R<digits>\n framing, from_env fd validation (reject stdio/non-pipe), signal-during-boot handling, and pid-file cleanup on post-claim boot failure.

Declined (with rationale in the assessment): reworking the double-fork-in-pre_exec / atfork hazard — pre-existing architecture, out of this issue's scope; the 120s backstop converts a hypothetical atfork hang into a bounded failure. Worth a separate follow-up if we want to move daemonization into a dedicated helper.

Secondary finding (run-create-back-to-back-no-supervisor): the readiness pipe REMOVES the confirmation-timeout ambiguity that could make a second, still-mid-poll `run create` look transiently supervisor-less. It does NOT by itself explain a *lost stdout envelope* if the root cause was shell backgrounding of two concurrent creates (the prior investigation found no code-level race). Not force-closing that tracker; leaving it for a fresh repro.
