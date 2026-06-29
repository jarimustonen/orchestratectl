---
created: 2026-06-29
updated: 2026-06-29
type: improvement
status: open
priority: normal
---

# merge.sh $TMUX_PANE cleanup should use worktree-path recovery (parity with supervisor)

## Description

Follow-on from the `worktree-merge-orphans-tmux-window` fix (commit `bfd7bfb`). The supervisor's `cleanup_terminal_nodes` path now recovers a renamed / detached-HEAD tmux window by worktree path, but `merge.sh` (the SKILL-side helper invoked by the agent inside its worktree) still captures the target window from `$TMUX_PANE` at runtime. A retry issued from a different window after a conflict-then-resolve cycle therefore targets the WRONG window.

Two equivalent fixes:

1. Teach `merge.sh` the same worktree-path → window-id lookup the supervisor uses, so a retry from any pane finds the right window.
2. Defer window teardown entirely to the supervisor and have `merge.sh` no longer touch tmux — simpler, single source of truth.

Prefer (2). The supervisor is already the canonical actor and bfd7bfb made its cleanup rename-proof; having two actors race on the same teardown is the bug class that motivated bfd7bfb in the first place.
