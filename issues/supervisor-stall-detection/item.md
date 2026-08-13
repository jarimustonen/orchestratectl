---
created: 2026-08-11
updated: 2026-08-13
type: bug
status: open
priority: normal
labels: [defer-0.2.1]
---

# supervisor reports stalled:false through a multi-hour silent agent hang; run wait default timeout (6h) is too long to surface it

## Description

## Incident (observed cutting ossctl 0.3.0, 2026-08-11)
A spinoff worker (kind=spinoff, run 01kzpfge026148xychxjr1v3w9) hung: the agent process stayed ALIVE (ps showed the `claude` process, STAT S+) but emitted **zero events beyond the 3 spawn events (run.created / node.created / supervisor.started) and made ZERO commits for ~6 hours**. The issue it owned never flipped to in-progress. Throughout, `orchestratectl run show` and `run wait` reported **`stalled: false`** and node `status: pending`. The supervisor (pid alive) sat idle the whole time. The hang was only surfaced because `run wait` finally hit its **default 6h timeout**. Cancelling the run (`run cancel`) cleaned it up correctly and a fresh re-spawn landed clean.

## Problem 1 (bug) — no stall detection
A supervisor should detect an agent that has emitted **no events for N minutes** (event-log silence, not just process-liveness) and mark the run `stalled: true`, so `run show`/`run wait` surface it. Process-alive is NOT progress — a wedged network call (LLM/registry/git) keeps the process alive indefinitely with no output. Today `stalled` stayed false through a 6h dead hang.

## Problem 2 (related, smaller) — `run wait` default timeout too long
`run wait`'s default timeout is ~6h (waited_ms ~21.6M in the incident). For interactive orchestration that means a hung worker burns the entire window before the caller learns anything. Consider a shorter sensible default, or document loudly that callers should pass `--timeout` for interactive use. (Workaround used: `--timeout 5400` on the retries got fast feedback.)

## Suggested acceptance
- [ ] Supervisor marks a run `stalled: true` after a configurable event-silence threshold (heartbeat/last-event-age based), even while the agent process is alive.
- [ ] `run wait`/`run show` surface the stall (and `run wait` can optionally return non-zero on stall).
- [ ] `run wait` default timeout reconsidered, or the `--timeout` guidance documented.

## Repro
Any worker that wedges early (before its first commit) — e.g. a hung network call — reproduces the false `stalled: false`. Filed from the ossctl 0.3.0 cut session.

## Decisions

### 2026-08-13T11:10:30Z · @adr-decision-2

DEFER-to-0.2.1: Supervisor-liveness bucket — a silent-hang is detected by the supervisor lease. The clean answer is the pi.dev self-report/lease plugin (0.2.1), not the 0.2.0 thin core. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
