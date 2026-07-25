---
created: 2026-07-25
updated: 2026-07-25
type: bug
status: open
priority: normal
---

# run create --headless: intermittent tmux-window-not-found race (worktree made, then clean rollback; retry succeeds)

## Description

During a real /stint (2026-07-25), 'orchestratectl run create --kind spinoff --headless' intermittently FAILED at creation with a LOUD error, then cleanly rolled back. Distinct from @supervisor-spawn-fails-silently-at-run-create (that one hangs/times-out and orphans a pending run with a dead supervisor and NO cleanup). Here the failure is immediate, self-cleaning, and retry succeeds.

REPRO: happened 2x this session, each time on a spawn issued shortly after another spinoff into the same 'headless' session (near-simultaneous / loaded headless session).

OBSERVED (stderr, non-envelope -> wrapped as create_sh_error_create_sh_unparseable):
  Targeting tmux session: headless
  Forking branch from base: main
  Creating worktree 'wt/01kyadb7rn-...'...
  ✓ Successfully created worktree and tmux window for 'wt/01kyadb7rn-...'
  Copying prompt to history/.worktree/...
  { error: code=tmux-window-not-found,
    message=No tmux window for 'wt/01kyadb7rn-...' (or flat 'wt-01kyadb7rn-...') in session 'headless',
    expected=a window named after the branch in session 'headless' }
  Cleaning up partial state (exit 1)...

So create.sh reports 'Successfully created ... tmux window', but the immediately-following window lookup in session 'headless' does NOT find it -> errors and rolls back the (already-created) worktree+branch. Partial state IS cleaned up (worktree/branch gone, verified via 'git worktree list'). Retrying the identical command a few seconds later succeeds.

EXPECTED: the window-created step and the window-lookup step should not race; either wait/poll for the window to appear (it seems to exist momentarily later) or make creation+lookup atomic. Intermittent -> likely a tmux settle/timing race when the headless session is under concurrent window creation.

WORKAROUND used: wrap 'run create' in a 3x retry with a few-seconds sleep; succeeds on attempt 2.

ENV: orchestratectl on main; macOS (darwin); rootless spawns into detached 'headless' tmux session; multiple parallel /stint + other sessions' worktrees present in 'headless' at the time.
