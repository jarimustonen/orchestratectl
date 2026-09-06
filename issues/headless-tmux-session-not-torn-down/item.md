---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: fixed
priority: normal
commits:
- hash: 9bdadff693a362bd4ed0fc99b44e09ccd60d7a7b
  summary: tear down empty headless tmux session after last managed window (Option 1, supervisor teardown)
closed: 2026-06-29
---

# Empty tmux 'headless' session left behind after last managed window removed

## Description

Reported 2026-06-29 from a deutschpad UI-redesign run using `taskfleet
run create --kind spinoff --headless`. After all spinoff runs went terminal
and the supervisor tore down each worktree, branch, and tmux window
cleanly, the parent `headless` tmux session itself lingered with a single
default `zsh` window:

```
$ tmux ls
default: 4 windows (attached)
headless: 1 windows (created Mon Jun 29 20:13:39 2026)   # <-- leftover

$ tmux list-windows -t headless
1: zsh* (1 panes) ... (active)
```

All three run dirs were terminal (`manifest.status = done`, supervisor
exited `reason: work-complete`).

## Severity

Low — cosmetic / minor resource leak. No data loss, no orphaned
worktrees/branches. But over batches the empty `headless` sessions
accumulate and confuse the user ("extra empty headless session in the
deutschpad folder").

## Root cause (suspected)

The supervisor's teardown path (`crates/taskfleet-cli/src/supervise/cleanup.rs`)
correctly removes the taskfleet-owned tmux window via
`tmux kill-window`, but does not check whether that was the last
taskfleet-managed window in the session. When a `--headless` /
`--tmux-session <name>` session was newly created by the first
`run create`, tmux automatically opens a default shell window in it
(`zsh` here); subsequent taskfleet runs add their own windows
alongside. After teardown of all taskfleet windows, the default
`zsh` window keeps the session alive.

## Expected behaviour

One of:

1. **Teardown also kills the empty session.** When the last
   taskfleet-managed window in a `--headless` / `--tmux-session`
   session is removed, the supervisor should also kill the session if
   the only remaining windows are the synthetic default shell
   (heuristic: window name `zsh`/`bash` with no taskfleet
   metadata).
2. **Never create the default window in the first place.** When
   taskfleet creates the session, immediately replace or kill the
   bootstrap window so the session only contains taskfleet windows.
   Then the session naturally dies when `tmux kill-window` removes the
   last one.

Option 2 is cleaner — no heuristics — but requires changing the session
bootstrap path. Option 1 is a localized fix in cleanup.

## Workaround

`tmux kill-session -t headless` once all headless runs are terminal.

## Repro

```bash
taskfleet run create --kind spinoff --headless --task '...'
# let it self-merge
tmux ls   # observe leftover empty `headless` session
```

