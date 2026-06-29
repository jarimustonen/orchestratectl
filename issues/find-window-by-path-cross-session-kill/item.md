---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: open
priority: high
---

# find_window_by_path can kill an unrelated tmux pane that cd'd into the worktree

## Description

`crates/octl-cli/src/supervise/cleanup.rs:268`'s `find_window_by_path` runs `tmux list-windows -a` (ALL sessions) and returns the **first** window whose `pane_current_path` either equals the spinoff's `worktree_path` or starts with `worktree_path + "/"`. The first match wins regardless of session, regardless of whether the window is actually the spinoff's.

If any unrelated pane (the user's main work pane, a `/worktree-code` review pane in another session, a sibling spinoff's pane briefly cd'd in for inspection) happens to have its cwd inside the spinoff's worktree at the moment cleanup runs, that pane's window gets killed by `tmux kill-window`. The user loses an unrelated session.

This is the dual of `worktree-merge-orphans-tmux-window` — the original window-by-name lookup was too narrow (rename-blind); this fix swung too wide.

## Repro (manual)

1. Spawn a spinoff: `orchestratectl run create --kind spinoff --title repro --task "echo hi" --headless`.
2. From any unrelated tmux pane, `cd /Users/<you>/Sources/<proj>__worktrees/wt-<short>-repro`.
3. Let the spinoff merge (or `run merge` it).
4. The unrelated pane's window is killed.

## Fix options

1. **Constrain by session.** Look up windows only in the supervisor's recorded session (manifest/run-record carries the parent-session name when set). Falls back to the renamed-window case as long as the rename happened inside the same session.
2. **Verify before killing.** After locating a candidate window, check its name still matches `wt/<short>-<slug>` or its worktree-path is exactly the spinoff's (not a sub-path) AND it has no extra panes attached by the user. Skip kill on mismatch.
3. **Match exactly, not by prefix.** Drop the `path.starts_with(&prefix)` clause so a sibling pane that cd'd one level deeper into the worktree (`worktree/src/...`) does not match. This narrows the surface but doesn't eliminate it (a pane at the worktree root still matches).

Prefer (1) — the supervisor knows which session owns its windows; querying that one session is both safer and faster than `-a`. Combine with (3) for defence in depth.

Surfaced 2026-06-29 during the B-fix campaign while debugging an apparent master-session disconnect (the master was not actually cd'd into the worktree this time, so cleanup did not hit the bug, but the path-traversal in `find_window_by_path` makes the failure mode trivially reachable).
