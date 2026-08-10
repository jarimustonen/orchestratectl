---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: open
priority: normal
labels: [run-wait, supervisor, reliability]
---

# run wait still blocks on orphaned run with node_count>0 (supervisor died mid-run)

## Description

## Description

`run wait`'s stillborn detection (issue `run-wait-stillborn-run-not-detected`)
only catches runs whose supervisor died *before* creating any worker node
(`node_count == 0`, `updated_at == created_at`). It deliberately does NOT catch
a run whose supervisor died *after* creating `n-0001` but before rolling the run
up to a terminal status.

Such a run has `node_count > 0`, a dead supervisor, and `status: pending` (or
`running`) — it can never reach a terminal status on its own, yet `run wait`
still blocks the full `--timeout` on it. This is the broader "orphaned run"
class that the stillborn fix scoped out (raised by an LLM reviewer during the
stillborn review; see `history/review-stillborn-run-detection.md`, finding #10).

## Why it's harder than stillborn

- A `node_count > 0` pending/running run is the NORMAL shape of a healthy,
  actively-working run. Distinguishing "supervisor dead, work stranded" from
  "supervisor alive, still working" needs the supervisor-liveness probe AND a
  guard against the create window / transient states — much closer to a real
  liveness/heartbeat signal than the unambiguous stillborn signature.
- Production has a supervisor-side backstop (`NO_WORKER_TICKS` + `NO_WORKER_GRACE`
  in `supervise/mod.rs`) but it only fires while the supervisor is ALIVE; a dead
  supervisor never runs it — which is exactly this case.

## Suggested approach (to validate)

- In `run wait`/`run show`, extend detection to `supervisor.alive == false &&
  status in {pending, running}` with a bounded idle window (like the orchestrate
  `is_stalled` grace) so a briefly-unschedulable supervisor isn't misread.
- Consider `run reattach`-then-recover as the remediation the hint points to
  (a reattach revives the supervisor, which then rolls the run up or fails it
  via the no-worker/agent-death backstops).

## Context

Spun off from the stillborn-run fix. Read-time detection preferred (no
reducer/schema change), mirroring the stillborn + orchestrate-stall hints.
