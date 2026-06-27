---
created: 2026-06-27
updated: 2026-06-28
type: chore
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
---

# Supervisor robustness pack: atomic PID claim + detach-reap + watchdog lock

## Description

Closes supervisor-pid-claim-race, supervisor-child-detach-reap, supervisor-watchdog-lock-retry in one coherent supervise/ change.

## Closure

Done. Three coordinated fixes landed on branch `supervisor-robustness-pack`:

1. **Atomic PID claim** (`pid_file::claim_pid_atomic` under the run flock) —
   closes `supervisor-pid-claim-race`.
2. **setsid + double-fork detach** in all three supervisor spawn sites, plus
   cascade SIGTERM to children on run-dir-vanished shutdown — closes
   `supervisor-child-detach-reap`.
3. **Lock-aware watchdog synthesis** (re-read `last_report` under the run
   flock, synthesize via `append_and_apply_unlocked`) — closes
   `supervisor-watchdog-lock-retry`.

Quality bar green: build + full workspace tests (18 binaries) + clippy +
fmt clean; no zombie/leaked supervisors. New tests: concurrent atomic claim,
legacy pid migration, out-of-range pid guard, watchdog defer, SIGHUP
survival. A multi-model `/llm-review` (Gemini, GPT-5.5, Opus, DeepSeek) ran;
all consensus must-fixes applied (pid bounds-check, identity-aware readback,
non-blocking child readback, stdin null, cascade union). See `handoff.md` for
deferred follow-up candidates (readiness-pipe handshake, `--force-claim`,
Docker PID-1 reaping).
