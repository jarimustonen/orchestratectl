---
created: 2026-06-27
updated: 2026-06-29
type: bug
status: fixed
priority: normal
closed: 2026-06-29
---

# worktree-merge: tmux window orphaned when a rebase conflict is resolved manually

## Description

When `taskfleet run merge` hits a rebase conflict, it exits `merge_failed`
and submits **no** terminal report — the node stays live and the supervisor
keeps polling. The user then resolves the conflict by hand (`git rebase
--continue` / `/complex-rebase`) and re-runs `run merge`. On that successful
retry the terminal report lands, but the tmux window is left open: the user has
to `tmux kill-window` it manually.

## Root cause

There are two independent actors that close the window, and the manual-resolution
path defeats **both** while nothing records the miss.

1. **merge.sh's own detached cleanup** keys off `$TMUX_PANE` — the pane the
   command was executed *from* — captured at runtime
   (`crates/taskfleet-cli/scripts/merge.sh`, the `tmux display-message -t "$TMUX_PANE"
   -p '#{window_id}'` line). In the normal autonomous flow the agent runs
   `run merge` inside its own spawned window, so this resolves to that window.
   But when a human resolves the conflict and re-runs the merge, the retry is
   frequently issued from a *different* window (the main/control window), so
   merge.sh either kills the wrong window or captures the wrong id — the spinoff
   window survives.

2. **The supervisor's cleanup** (`crates/taskfleet-cli/src/supervise/cleanup.rs`,
   `cleanup_terminal_nodes` → `cleanup_node` → `kill_tmux_window`) targets the
   window by the spawn-time `tmux_identity` (the stable `@NNNN` window id) when
   present, and otherwise by the legacy bare window **name**
   (`Node::tmux_window`). The name path is the fragile one: a manual rebase
   commonly leaves the worktree on a detached HEAD mid-rebase and triggers
   tmux's automatic window rename, so the recorded `wt/<short>-<slug>` name no
   longer matches. The old code issued `tmux kill-window -t <name>` leniently
   and **swallowed every failure** — so a name that no longer matches was a
   silent no-op. Worse, nothing was recorded: the run still rolled up to `done`,
   masking the orphan entirely. (For modern spawns that carry a qualified
   `tmux_identity` the id is rename-proof, so the gap is widest for legacy nodes
   and for the merge.sh actor — but the *silent* failure mode applied to both.)

So in the manual-resolution case neither actor is guaranteed to close the
spinoff window, and the silent swallow made every recurrence invisible.

## Fix

`crates/taskfleet-cli/src/supervise/cleanup.rs` (`close_tmux_window`):

- The recorded-target kill is still attempted unconditionally first (we never
  precheck with `list-windows` — a transient empty list must not skip a real
  kill; same hard-won rule merge.sh follows). `run_lenient` now returns whether
  the kill succeeded.
- **Root-cause recovery:** when that kill reports the target missing, re-find the
  window by the node's `worktree_path` via
  `tmux list-windows -a -F '#{window_id}\t#{pane_current_path}'` and kill the
  window whose pane is still parked in the worktree. The pane cwd is rename-proof
  — a manually-resolved rebase mutates the branch/window name but not where the
  pane sits — so this closes the renamed/detached-HEAD orphan directly.
- **Defensive audit fallback:** if even the path lookup finds nothing, append a
  non-fatal `cleanup.window_missing` event (node id, attempted window, lookup
  method, worktree path) under an idempotency key, then continue. Cleanup never
  fails the run — but the orphan is now visible in the run log instead of silent.

`crates/taskfleet-core/src/reducer.rs` lists `cleanup.window_missing` explicitly as an
append-only audit record that folds to a clean no-op (it mutates no projection;
the event log is its only home), alongside `orchestrator.decision` /
`discuss.critical`.

## Tests

- `crates/taskfleet-cli/src/supervise/cleanup.rs`:
  - `window_killed_by_id_no_fallback` — happy path issues no probe / no event.
  - `missing_window_records_event_and_does_not_fail` — the "window already gone"
    path records exactly one `cleanup.window_missing` and does not panic/fail.
  - `renamed_window_recovered_by_worktree_path` — a renamed window is recovered
    by worktree path and killed, with no audit event.
  - `missing_window_event_is_idempotent` — a second cleanup pass appends no
    duplicate (idempotency key).
- `crates/taskfleet-cli/tests/supervise_gates.rs::missing_window_records_event_without_failing_run`
  — end-to-end through `supervise --once`: an orphaned window records the event
  yet the run still rolls up to `done`.

## Reproduction note

A live end-to-end reproduction (real tmux + workmux + a spinoff hitting a real
rebase conflict) is impractical inside this worktree without spawning real
supervisor/tmux state and risking orphaning *this* session's windows. The
failure was instead reproduced at the unit/integration level by stubbing `tmux`
so `kill-window` reports the target missing and `list-windows` shows (or hides) a
pane in the worktree — exercising both the recovery and the audit-fallback code
paths the issue calls for.
