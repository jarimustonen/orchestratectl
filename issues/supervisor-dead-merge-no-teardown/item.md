---
created: 2026-07-03
updated: 2026-07-03
type: bug
status: fixed
priority: high
commits:
- hash: 979b794
  summary: 'fix(run): ensure live report consumer on merge + supervisor liveness on show/list'
closed: 2026-07-03
---

# run merge reports success when supervisor is dead; teardown silently never happens

## Description

**Reporter:** homebase worktree `wt/01kwj3494b-bridge-issue-comment` session
**Reported:** 2026-07-03
**Version:** orchestratectl `0.0.2-alpha` (commit `65a21007f787e3a986f455cdcaf5165272c9cd66`)
**Severity:** Medium — no data loss (the merge lands correctly on `main`),
but the worktree + tmux window + branch are silently left behind and the
success envelope misleads the caller into telling the user cleanup
happened.

## Summary

`orchestratectl run merge` performed the rebase+merge and wrote the
terminal `node.report`, then returned `{"merged": true, "report_seq": 5}`.
But the run's per-run **supervisor process had already been killed by
SIGTERM ~1h11m earlier**, so nothing consumed the terminal report.
Teardown (close tmux window, remove worktree, delete branch) never ran,
and `manifest.status` stayed at `pending`. The command reported success
anyway, with no signal that the supervisor was dead.

## Evidence — run `01kwj3494bvaskz6y1x7482mjn`

`events.jsonl` (UTC):

| seq | ts (UTC)             | kind                | data |
|-----|----------------------|---------------------|------|
| 1   | 2026-07-02 18:58:13  | `run.created`       | kind=code, lifecycle=interactive, source_branch=main |
| 2   | 2026-07-02 18:58:16  | `node.created`      | agent_pid=57223, branch=wt/01kwj3494b-bridge-issue-comment |
| 3   | 2026-07-02 18:58:16  | `supervisor.started`| **pid=57322** |
| 4   | 2026-07-03 06:53:01  | `supervisor.exited` | **pid=57322, reason=signal, signal=SIGTERM** |
| 5   | 2026-07-03 08:04:30  | `node.report`       | terminal merge report (via explicit-merge) |

- `supervisor.stderr.log`: `{"pid":57322,"reason":"signal","iterations":53471}`
- `ps -p 57322` → dead (confirmed).
- `manifest.json`: `status: "pending"`, `applied_seq` frozen at the
  supervisor's last-applied event; `worktree_root: null`,
  `managed_tmux_session` recorded.
- `run merge` output:
  `{"schema_version":1,"data":{"run_id":"01kwj3494bvaskz6y1x7482mjn","node_id":"n-0001","branch":"wt/01kwj3494b-bridge-issue-comment","source":"main","merged":true,"report_seq":5}}`

**Timeline:** supervisor SIGTERM'd at 06:53:01 UTC (≈09:53 local);
`run merge` ran at 08:04:30 UTC. Between them the run had a dead
supervisor and no watchdog noticed. Likely SIGTERM source: the hosting
tmux server / session cycling on hauis (its tmux server hosts the `tw`
sessions; a restart SIGTERMs children).

## The two defects

### 1. `run merge` returns success without ensuring a live consumer of the terminal report

The command writes `node.report` and returns `merged: true`
optimistically. When the recorded supervisor PID is dead, the report is
never consumed and teardown never fires — yet the caller is told it
succeeded. The documented contract ("the supervisor closes the tmux
window… within a second or two") silently does not hold.

**Suggested fix (pick one):**

- Before returning success, check the recorded supervisor PID. If dead,
  either (a) perform teardown inline / auto-`reattach`, or
  (b) return `merged: true` **with a warning** in the envelope
  (e.g. `warnings: ["supervisor not running; teardown deferred — run
  \`orchestratectl run reattach <id>\`"]`) so the caller can recover
  instead of reporting a clean close.
- At minimum, surface `supervisor_alive: false` in the `run merge`
  envelope.

### 2. The supervisor is a single unsupervised process with no liveness detection / auto-restart

One SIGTERM orphans the run's whole lifecycle permanently. `run reattach`
exists as manual recovery, but nothing detects the dead supervisor and
invokes it, and `run show` gives no hint the supervisor is dead
(`status` just freezes at `pending`). A run can sit orphaned for hours.

**Suggested fix:**

- `run show` (and `run list`) should report supervisor liveness — e.g.
  a `supervisor: {pid, alive}` field, and flag `status: pending` +
  dead PID as a distinct "orphaned" condition.
- Consider auto-reattach on any lifecycle-touching command (`merge`,
  `show`, `cancel`) when the recorded PID is dead, or a lightweight
  watchdog/systemd-style restart for the supervisor.

## Workaround used

Hold off on teardown; run `orchestratectl run reattach
01kwj3494bvaskz6y1x7482mjn` to restart the supervisor — it then
consumes seq 5 and performs the deferred teardown correctly.

## Acceptance criteria for a fix

- `run merge` (and any other lifecycle-touching command) either
  auto-reattaches when the recorded supervisor PID is dead OR returns
  `merged: true` with a `warnings: [...]` entry naming the deferred
  teardown + the exact recovery command. Silent success is not
  acceptable.
- `run show` / `run list` surface supervisor liveness (`supervisor: {pid,
  alive}`) so a caller can distinguish "still working" from "orphaned".
- Integration test in `crates/octl-cli/tests/` (extending
  `e2e_spinoff.rs` or a new file) that kills the supervisor mid-run,
  then invokes `run merge`, and asserts the envelope carries the
  warning / performs recovery — no silent success.

## Note

The actual work merged fine: all 8 commits are on `main` (rebased).
This bug is purely about the post-merge cleanup step and its
misleading success reporting.

## Source

Original report was at `BUG-REPORT-supervisor-dead-merge-no-teardown.md`
at repo root (2026-07-03). Migrated to issuectl and the standalone file
removed.

