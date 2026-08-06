---
created: 2026-08-05
updated: 2026-08-06
type: bug
status: fixed
priority: normal
closed: 2026-08-06
---

# orchestrate driver run created but never driven becomes a silent zombie

## Description

# `--kind orchestrate` driver run created but never driven becomes a silent zombie

_Found during an issuectl `/stint` where a prior session launched an intake campaign._

## Observed

A previous session ran `orchestratectl run create --kind orchestrate …` and then
stopped without driving the fan-out loop. **15 hours later** the run was still a
zombie:

- driver node `n-0001`: `updated_at == created_at`, status `pending`, `children: 0`
- `supervisor.state.json`: `spawned_children: {}`, `last_seq_own: 3`
- `events.jsonl`: only `run.created` → `node.created` → `supervisor.started`, then silence
- no integration branch, no tmux window
- **yet the supervisor process was alive the whole time** (`supervise <run-id>`, 15h32m)

The `--kind orchestrate` supervisor only *adopts children*; it does not itself
drive the fan-out. So a driver run whose orchestrator agent never runs §4 (or dies
immediately) sits forever: alive, doing nothing, indistinguishable at a glance from
a healthy long-running campaign (`run show` just says `pending`).

## Expected

Some signal that a driver run is stalled/undriven, e.g.:

- `run show` / `run list` flag a `--kind orchestrate` run whose driver node has been
  `pending` with 0 children and no new events for > N minutes (a `stalled: true` hint
  or a distinct status), so `orchestratectl run list` doesn't show it as an ordinary
  live run; and/or
- the supervisor emits a heartbeat/`driver.idle` warning when it has adopted no
  children within a grace window.

## Impact

Silent 15h no-op; the operator only discovered it by manually reading
`events.jsonl` + `supervisor.state.json`. The fix this time was `run cancel` +
relaunch with an agent actively driving the loop.

## Workaround

After `run create --kind orchestrate`, confirm progress from the event log (expect
`child.spawned` shortly), not just `run show` status; if the driver node stays
`pending` with 0 children, the run is not being driven — cancel and relaunch with an
active orchestrator.
