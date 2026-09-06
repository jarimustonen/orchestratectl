---
created: 2026-08-18
updated: 2026-08-22
type: bug
reporter: jari
status: wontfix
priority: normal
provenance: agent-homebase-wrapup
lane: lifecycle
lane_seq: 35
closed: 2026-08-22
closed_by: jari
---

# Worker that exits without run merge is indistinguishable from a healthy…

## Description

Worker that exits without run merge is indistinguishable from a healthy pending run

Observed (taskfleet 0.2.2, issuectl repo, 2026-08-17, run 01m07e2m4nxsmm6wqqtcdsybh5):
a spinoff worker completed Phase A of its task, printed "I stopped before Phases B-D, so I did
not close or merge the run" in its pane, and went idle WITHOUT calling `taskfleet run merge`.
From the orchestrator's side this state was invisible:

- `taskfleet run show <id> --output json` reported `status: pending`, `lifecycle: autonomous`
  and, notably, `nodes: []` (empty array) even though node n-0001 existed and had run.
- No attention/stall signal anywhere in `run show` or `run list`. The `run salvage` help text
  describes an `attention-required` state ("a worker that exited cleanly but skipped run merge"),
  but nothing surfaced that state for this run.
- Diagnosis required reading the tmux pane by hand (tmux capture-pane) to see the worker's shell
  prompt / farewell message. Recovery worked (tmux send-keys nudge; salvage was the fallback),
  but detection was entirely manual.

Expected:
- The supervisor detects "worker process exited / went idle but node not terminal" and surfaces it:
  run status or a flag like `attention_required: true` in `run show`/`run list`, so a caller
  polling the run (or `run wait`) learns the run is stuck rather than healthy-pending.
- `run show` should list the node(s) in `nodes` while the run is live — the empty `nodes: []` on a
  run with a live worker made even manual triage harder.

Impact: a stalled autonomous run looks identical to a slow one; callers wait on `run wait`
indefinitely and a human must inspect tmux panes to notice.

## Resolution

### 2026-08-22T17:37:26Z · @jari

The current product already records worker.exited and surfaces a clean worker exit without run merge as attention_required. No separate implementation is wanted; close the historical symptom rather than rebuilding it.
