# supervisor-robustness-pack — handoff

Closes `supervisor-pid-claim-race`, `supervisor-child-detach-reap`,
`supervisor-watchdog-lock-retry` in one coherent `supervise/` change.

## What shipped

1. **Atomic PID claim** — `pid_file::claim_pid_atomic` holds the run flock
   across read-existing-pid → liveness/identity check → write-our-pid, so
   two concurrent `supervise`/reattach-spawned supervisors can't both claim a
   run. Used by `supervise` startup. `run reattach` keeps a best-effort
   stale-pid pre-check; the real guarantee is the spawned supervisor's atomic
   claim (every supervisor — direct or reattach-spawned — funnels through
   `dispatch` → `claim_pid_atomic`, so exactly one wins the flock).
2. **setsid + double-fork detach** — all three spawn sites share
   `supervisor_spawn::{detached_supervise_command, spawn_and_reap}`. The
   grandchild reparents away from the spawner; the intermediate is reaped by
   the spawner → no zombies, and `kill(pid,0)` never sees a zombie-as-alive.
3. **Lock-aware watchdog synthesis** — `watchdog_tick` re-reads the node
   under the run flock before synthesizing a terminal `node.report`, deferring
   to a live report that landed in the window.
4. **Cascade SIGTERM** — on run-dir-vanished self-terminate, the supervisor
   SIGTERMs every identity-verified tracked child (union of `spawned_children`
   and reseeded `child_tails`).

Multi-model review fixes applied (see `history/review-supervisor-robustness-pack.md`):
pid bounds-check before any `kill`, identity-aware pid readback, non-blocking
child-pid readback (no parent-loop stall), stdin→/dev/null, cascade union,
`spawn_and_reap` error logging, legacy-lockout message.

## Deferred — candidate follow-up issues

These were raised in review and consciously deferred (not regressions; mostly
pre-existing or architectural):

- **Readiness-pipe startup handshake** (spin-off candidate). Pid-file polling
  can't cleanly distinguish "booting" / "already running" / "exec failed" /
  "claim lost". An inherited pipe where the supervisor writes
  `READY <pid>` / `ALREADY_RUNNING` / `ERROR` would make spawn readiness and
  the real pid deterministic. Larger change; current polling is adequate.
- **`--force-claim` escape hatch** (spin-off candidate). A *legacy* (no
  start-time) pid file whose pid has been recycled to a live unrelated process
  blocks startup until a human removes the file. We now emit a clear message;
  an explicit force/auto-stale heuristic would close it fully.
- **Docker PID-1 reaping** (doc). If `orchestratectl` is itself PID 1 in a
  container with no init/reaper, double-forked grandchildren that exit become
  zombies (init==us, and we have no `waitpid(-1)` reaper loop). Document the
  `tini`/`dumb-init` requirement, or add a SIGCHLD reaper when running as PID 1.
- **`SA_RESTART` vs hung flock** (note). With `SA_RESTART`, a supervisor
  blocked in `RunLock::acquire` on a stale/hung lock can't be killed by
  SIGTERM (the syscall restarts). This is a deliberate existing trade-off
  (clean shutdown during the append `flock`/write); changing it risks the
  §7.8 clean-shutdown contract. Left as-is.
- **`remove_if_owner` at shutdown is unlocked + pid-only** (low risk). The
  window is benign (the exiting supervisor is still alive while removing, so a
  racing claimant sees it live and backs off). Could be made
  flock+identity-aware for consistency.
- **`CHILD_DIR_WAIT` (pre-existing, D1)** still blocks `spawn_child_supervisor`
  up to 5s waiting for the child run dir. Not introduced here; the new pid
  readback stall *was* removed. Could be moved off the parent tick alongside
  the readiness-pipe work.
- **Test depth**: the watchdog test exercises the static "report already
  present → defer" case; the true read-vs-synthesize race needs a fault hook
  (V2 covers happy-path synthesis-under-lock). A multi-process claim-race test
  would belt-and-suspender the threaded one (flock is per-fd, so the threaded
  test is valid today).

## Blocker escape hatch — NOT triggered

The double-fork did **not** interact badly with tracing-subscriber: the
second `fork` runs in `pre_exec`, *before* `exec`, so no supervisor runtime
threads exist yet to be orphaned — the grandchild builds its tracing stack
fresh after `exec`. Logs flow normally (verified: no lost-log symptoms; the
`run-dir-vanished` self-terminated event and supervisor.stderr.log both land).
