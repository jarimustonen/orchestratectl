---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: open
priority: normal
---

# Supervisor never completes run on agent-submitted terminal node.report

## Description

Symptom: an autonomous-kind worker submits a terminal `node report` (success or failure), the node correctly reaches `status: done`, but the **run** never transitions to a terminal status and the per-run supervisor process never exits. `orchestratectl run show` keeps reporting `status: pending` indefinitely and the supervisor keeps polling — the exact dangling symptom of `spinoff-must-submit-node-report`, which the SKILL fix alone does NOT resolve.

Verified live 2026-06-28 on run 01kw7btqhpdgjeh55zga7wghjs (the spinoff implementing the SKILL fix):
- Submitted a valid `node.report` (event seq 4). `node show n-0001` -> `status: done`, `last_report` populated. Correct.
- `run show` -> manifest `status: pending` (unchanged) for 18+ minutes.
- Supervisor PID 75074 stayed alive the whole time; supervisor.stderr.log empty.

Root cause (read from source):
- `octl-core/src/reducer.rs::reduce_node_report` updates only the NODE projection. It does not roll the terminal node up to the run manifest status, and emits no `run.status` event.
- The supervisor's `all_work_done` (`octl-cli/src/supervise/mod.rs`) keys off the **manifest** `status` being `Done|Failed|Cancelled`. Nothing sets it for the success path.
- `run.status` events are produced ONLY by `run cancel` (`octl-core/src/cancel.rs`). There is no producer on the agent-success / node.report path.
- The watchdog only synthesizes a terminal `node.report` for **non-terminal** nodes whose agent died (mod.rs ~line 1162 guards `!terminal`); an already-terminal node is skipped, so even after the agent process exits, no `run.status` is emitted.

Consequence: the only way a normal run reaches a terminal status today is `run cancel` (the documented workaround in `spinoff-must-submit-node-report`). The happy path (agent reports success) leaves the run `pending` and the supervisor alive forever.

Relationship to siblings:
- `spinoff-must-submit-node-report` (the SKILL fix) makes agents actually CALL `node report`. Necessary but not sufficient.
- `supervisor-close-tmux-on-terminal` assumes the supervisor already exits on a terminal report ("Supervisor PID ... died within 3 seconds (correct)") — but that observation was the `run cancel` path. On the node.report path the supervisor does NOT exit. THIS issue is the prerequisite: the run must first reach a terminal status from a terminal node before any terminal-transition handler (tmux close, clean exit) can fire.

Fix direction (pick one, decided by maintainer):
1. Reducer roll-up: when `reduce_node_report` terminalizes the last/only live node of a run, also transition the run manifest status (Done if all nodes succeeded, Failed if any failed). Pure projection, no new event — but then `run show` and `all_work_done` see it. Simplest; keeps the log as the source of truth via node states.
2. Event emission: have the supervisor (single arbiter) append a `run.status` event when it observes all of its nodes terminal, under the run flock (mirrors how `run cancel` appends one). More explicit and auditable; consistent with the existing `run.status` machinery the supervisor already consumes.

Recommend option 2 for symmetry with cancel and because the supervisor is already the single arbiter that owns run lifecycle transitions.

Acceptance:
- After a worker submits a successful `node.report`, `run show` reaches a terminal `status` (e.g. `completed`/`done`) within a few supervisor ticks and the supervisor process exits on its own.
- A failed report drives the run to `failed`.
- Existing `run cancel` behavior unchanged; no duplicate terminal `run.status` when both a report and a cancel race (reducer terminal-state guard already covers this).
- Integration test: spawn-free reducer/supervisor test asserting run terminal status after a node.report for a single-node run.
