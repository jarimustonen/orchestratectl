---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: fixed
priority: normal
closed: 2026-06-29
---

# tmux window lingers in headless session after run cancel

## Description

After `orchestratectl run cancel <id>` on a `--headless`-spawned run, the worktree and branch are removed but the tmux window stays in the detached `headless` session. The deployed `create.sh` does not emit the **qualified tmux identity** (`session:window_id` form) the supervisor needs to find the window when it lives outside the supervisor's own session.

Surfaced 2026-06-29 by the `headless-parent-session-rejected` fix-spinoff's smoke test: after a clean cancel, `tmux list-windows -t headless` still listed the throwaway window even though `git worktree list` and `git branch -l` were clean.

Likely fix: `create.sh` (in `homebase/dotfiles/src/.claude/skills/worktree/scripts/create.sh`) needs to emit the qualified tmux identity in its success envelope so the supervisor can address the window regardless of which session it lives in.

## Resolution

Root cause: create.sh's success envelope only emitted `tmux_window` (the bare
window *name*), `agent_pid_hint`, and `workmux_session`. It never emitted the
qualified identity triple (`tmux_session`, `tmux_window_id`, `tmux_socket`) that
the Rust side reads into `Node.tmux_identity`. So for a headless spawn the
supervisor had no `@NNNN`-on-socket handle, and its `run cancel` cleanup could
not address a window living in a session/server it doesn't own.

The Rust side was already complete — `SpawnOutcome` parses all three fields,
`create.rs` folds them into `node.created`, the reducer stores `TmuxIdentity`,
and `cleanup::close_tmux_window` prefers `tmux -S <socket> kill-window -t
<window-id>`, falling back to bare name then to a session-scoped
`find_window_by_path`. The only gap was the script not supplying the data.

Fix (homebase commit **5c47e72**): create.sh now emits the triple on EVERY
successful spawn — foreground and headless alike. `tmux_window_id` and
`tmux_session` were already in hand (`WORKMUX_SESSION` + the matched
`#{window_id}`); the socket is resolved with `tmux display-message -t
<window-id> -p '#{socket_path}'`, targeting the window explicitly so it works
even when create.sh runs outside tmux. An empty socket lookup serialises to JSON
`null`, which the Rust side already normalises.

Verification:
- `cargo test --workspace` green (one unrelated flaky timing test,
  `self_terminate_when_run_dir_vanishes`, passes in isolation).
- Manual smoke: `run create --kind spinoff --headless` recorded
  `tmux_identity {socket=/private/tmp/tmux-501/default, session=headless,
  window_id=@193}`; after `run cancel` the window vanished from the `headless`
  session within ~1s and the worktree + branch were clean.
