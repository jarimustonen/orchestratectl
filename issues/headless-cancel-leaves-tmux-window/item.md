---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: open
priority: normal
---

# tmux window lingers in headless session after run cancel

## Description

After `orchestratectl run cancel <id>` on a `--headless`-spawned run, the worktree and branch are removed but the tmux window stays in the detached `headless` session. The deployed `create.sh` does not emit the **qualified tmux identity** (`session:window_id` form) the supervisor needs to find the window when it lives outside the supervisor's own session.

Surfaced 2026-06-29 by the `headless-parent-session-rejected` fix-spinoff's smoke test: after a clean cancel, `tmux list-windows -t headless` still listed the throwaway window even though `git worktree list` and `git branch -l` were clean.

Likely fix: `create.sh` (in `homebase/dotfiles/src/.claude/skills/worktree/scripts/create.sh`) needs to emit the qualified tmux identity in its success envelope so the supervisor can address the window regardless of which session it lives in.
