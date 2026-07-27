---
created: 2026-07-26
updated: 2026-07-27
type: improvement
status: fixed
priority: normal
related: ['@capture-agent-output-to-run-dir']
commits:
- hash: 45c23d5
  summary: capture agent.log by stable pane_id, fall back to window_id
- hash: e153e28
  summary: apply llm-review findings (agent-pane binding, back-compat tests, capture_target guard)
closed: 2026-07-27
---

Follow-up spun off from `capture-agent-output-to-run-dir`.

The supervisor's agent-log capture arms `tmux pipe-pane -t <window_id>`, which
resolves to the window's **active pane**. For the autonomous headless path (the
capture feature's priority) the window has exactly one pane = the agent, so this
is correct today. But in an interactive `/worktree-code` session where the user
splits the window, the active pane may not be the agent's — so `agent.log` would
capture the wrong pane (or be empty), and the supervisor could record the user's
own shell output (mild privacy concern).

## Fix direction
Record a stable `pane_id` (`%NN`, `#{pane_id}`) at agent spawn and target it
directly in `pipe-pane -t %NN`. This touches:
- `octl-core` `TmuxIdentity` schema (add `pane_id`)
- `create.sh` (emit `#{pane_id}`)
- `run/spawn.rs` parsing of the create handoff
- `supervise/capture.rs` to prefer `pane_id` over `window_id`
- possibly watchdog identity handling for consistency

Out of scope for the additive capture diff (which must not touch the schema or
create.sh). Documented as a known limitation in `capture.rs` module docs.
