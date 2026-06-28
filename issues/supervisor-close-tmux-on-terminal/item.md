---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: fixed
priority: normal
commits:
- hash: ed99cc7
  summary: supervisor closes tmux window + removes worktree + deletes branch on terminal autonomous run
closed: 2026-06-28
---

# Supervisor does not close worker tmux window on terminal state (autonomous kinds)

## Description

Symptom: when an autonomous-kind run reaches a terminal state (the worker either submits a `node report` or the user runs `run cancel`), the per-run supervisor process exits — but the worker's tmux window is NOT closed. The user sees a tmux window with a quiescent Claude prompt and no obvious way to tell whether it is still doing something. Cleanup requires manual `tmux kill-window`.

First observed 2026-06-28 (haukinen) immediately after `orchestratectl run cancel 01kw79n2yv3epts3amfszmv3aa`. Supervisor PID 72879 died within 3 seconds (correct), but tmux window `default:6` (🎬 🚀 wt-01kw79n2yv-supervise-test-teardown-leak) stayed open. Same observation expected for any /worktree-spinoff that submits a terminal node.report once `spinoff-must-submit-node-report` (the SKILL fix in flight) lands — the SKILL change alone will not auto-close the window.

Expected behavior (autonomous kinds only):

When the supervisor observes that the run has transitioned to a terminal lifecycle (`completed`, `failed`, `cancelled`) AND the run's kind is autonomous (spinoff, orchestrated, research, make-skill, bugfix, technical-decision, fan-out child), it should:

1. Close the worker's tmux window via `tmux kill-window -t <socket>:<session>:<window>` (the qualified tmux identity is already stored in `node.tmux_identity` per the node.created event payload — design.md mentions tmux_socket / tmux_session / tmux_window_id fields).
2. Exit cleanly itself.

Interactive kinds (`code`, `orchestrate`) must NOT auto-close — the user owns the window for `/worktree-code` and the orchestrator agent IS the conversation for `/orchestrate`.

Edge cases:

- The tmux window may already be gone (user killed it manually). `tmux kill-window` on a missing window returns non-zero — treat as success, log at debug.
- The tmux socket may differ from the default if the run was started under a non-default workmux config. The qualified tmux identity in the node payload is the source of truth; do not assume `default:<name>`.
- The agent may still be doing post-work (e.g. running `/wrap-up` style content) when the report lands. Give it a small grace period (5-10s) before kill, OR rely on the SKILL contract that says "submit node.report as the LAST step" — then no grace period needed.
- A `run reattach` after the terminal state should NOT re-open the window.

Fix direction:

1. In `crates/octl-cli/src/supervise/` (the supervisor binary's main loop), add a terminal-transition handler. When the run becomes terminal AND `Kind::lifecycle()` returns `Autonomous`, look up `node.tmux_identity` from the manifest (or the latest `node.created` event) and call `tmux kill-window`.
2. Add integration tests that exercise both paths — terminal-via-report and terminal-via-cancel — and assert the tmux window is gone afterwards. The test fixture can stub tmux by intercepting the kill-window call (the supervisor should use a helper trait or a process spawn that the test can swap).
3. Document this in the SKILL.md "Following progress" section so the agent / user knows: "tmux window auto-closes for autonomous kinds when the run reaches terminal state; interactive kinds keep the window open until the user closes it themselves".

Workaround for already-dangling windows:

```
tmux kill-window -t <window>
git worktree remove <path>
git branch -D <branch>
```

Sibling issue: `spinoff-must-submit-node-report` — that fix gets the SKILLs to actually call `node report` at all. THIS issue is what makes the supervisor close the window once the report arrives. Both are needed for the user to see a clean "spawn → work → window vanishes" loop.
