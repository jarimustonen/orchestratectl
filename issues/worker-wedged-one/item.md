---
created: 2026-08-21
updated: 2026-08-22
type: feature
status: obsolete
priority: normal
closed: 2026-08-22
closed_by: jari
---

# A worker wedged on one long-running command is invisible to supervision

## Description

## Observed

A worker that wedges on a single long-running shell command is completely invisible to
supervision. There is no elapsed-time signal anywhere in the run state.

During an ossctl stint, two autonomous spinoffs each hung inside the `/assess-models` skill on
an unbounded `find /Users/jari …`:

| run | elapsed on one command |
|---|---|
| `01m0fathfnk4dexmz93kqnkeag` | **11 h 28 min** |
| `01m0fzj86qn21r4df5c8ksghg7` | **5 h 38 min** |

Throughout, `taskfleet run show` reported entirely healthy runs:

```
status = pending
stalled = False
attention_required = False
awaiting_input = False
supervisor = {'pid': 50551, 'state': 'alive', 'alive': True}
```

No `agent-died` event, no stall flag, supervisors alive. The event log had nothing after
`supervisor.started` — which is itself the tell, but nothing surfaces "no events for N hours"
as a signal. Two successive `run wait` calls timed out looking entirely normal.

The wedge was only discoverable by capturing the tmux pane by hand and reading the harness's
own elapsed counter (`Elapsed 41256.3s`). After killing the two `find` processes, both agents
resumed within seconds and merged **143 seconds** later — so the commands were the entire
delay, and roughly $9.14 and $6.63 of run cost was spent idling.

## Why this is worth a guard rather than a one-off fix

The specific cause (a skill locator running an unbounded home-wide `find`) is being fixed
separately in homebase (`assess-models-wedges`). But the *class* is general: any worker shell
command that blocks indefinitely — a network call with no timeout, a lock wait, a prompt on a
tool that ignores non-interactive mode — produces exactly this signature, and the supervisor
cannot currently tell it apart from a worker that is thinking hard. Long runs are legitimate
here (heavy review units genuinely run 54–96 min), so the guard has to be about *silence*, not
about total duration.

## Expected — one or more of

1. A per-command timeout for worker shell invocations (configurable, generous by default),
   so a wedged command fails loudly instead of silently consuming the run.
2. A stalled heuristic on **elapsed time without any new event**, surfaced through the
   existing `stalled` flag that today never fires for this case.
3. Surfacing the currently-executing command and its elapsed time in `run show`, so an
   orchestrator can see the wedge without attaching to tmux.

(3) alone would have turned an 11-hour mystery into a five-second diagnosis.

## Environment

taskfleet 0.4.1 (commit c15d6af4e12e728ce102a933ce17f9f4c2f18dee), macOS, `pi` harness.

## Close condition

Close when a worker blocked on a single command for hours is distinguishable from a healthy
one through `run show` / `run wait` alone, without attaching to the tmux pane.

## Resolution

### 2026-08-22T17:37:43Z · @jari

The incident remains valuable evidence, but the proposed remedies are superseded by @worker-telemetry-protocol. Preserve this report as the concrete long-command case; do not implement a separate silence heuristic or command-timeout feature from it.
