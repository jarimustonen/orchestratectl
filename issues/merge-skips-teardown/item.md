---
created: 2026-07-24
updated: 2026-07-25
type: bug
reporter: jari
status: fixed
priority: normal
labels: [worktree, merge]
closed: 2026-07-25
---

# run merge reports merged+terminal but leaves worktree, branch, and tmux window in place

## Description

`taskfleet run merge` returned success but did NOT tear down the worktree, branch, or tmux window. The per-run supervisor reached a terminal state without performing (or completing) the teardown it owns, leaving the worktree stuck.

## Environment
- taskfleet 0.1.0 (commit a54f0ff), macOS (gertrud, tw session)
- Run kind: the interactive orchestrator/`stint` worktree (window icon `💻`), branch `wt/01kxv2aak3-stint-review-skill-family`, source `main`.
- Run id: `01kxv2aak3wyh694yk5jm7ee6v`, node `n-0001`.

## Repro
1. Work in an interactive worktree run to completion; commit everything (tree clean).
2. Run `taskfleet run merge "$run_id" --report-file <valid §7.3 payload>`.
3. Observe the success envelope, then check the worktree / branch / tmux window.

## Expected
Per the `worktree-merge` skill contract: on a clean merge the supervisor "closes the tmux window, removes the worktree, and deletes the branch on the terminal transition. Nothing for you to clean up by hand."

## Actual — merge reported success, teardown did not happen
`run merge` returned:
```
{"data":{"run_id":"01kxv2aak3wyh694yk5jm7ee6v","node_id":"n-0001","branch":"wt/01kxv2aak3-stint-review-skill-family","source":"main","merged":true,"report_seq":7,"supervisor":{"state":"terminal"}}}
```
The merge itself succeeded (branch landed on `main`, verified in the canonical clone). But afterwards:
- **Worktree dir still present:** `/Users/jari/Sources/homebase__worktrees/wt-01kxv2aak3-stint-review-skill-family` still exists and still appears in `git worktree list`.
- **Branch still present:** `wt/01kxv2aak3-stint-review-skill-family` still exists (still checked out in the stuck worktree).
- **tmux window still present:** `default:4  🏠 💻 wt-01kxv2aak3-stint-review-skill-family`.
- **No supervisor process for this run:** `ps` shows supervisors for other live runs but none for `01kxv2aak3…` — the supervisor for this run has exited.
- **`taskfleet run show 01kxv2aak3wyh694yk5jm7ee6v` returns null `status`/`lifecycle`/`supervisor`** (the run record reads empty/terminal but the physical resources were never reclaimed).

So the terminal transition was recorded (`report_seq: 7`, `supervisor.state: terminal`) and the supervisor exited, but the teardown side effects (worktree remove, branch delete, tmux kill-window) did not run or did not complete. The user had to remove the worktree, branch, and window by hand.

## Impact
Silent resource leak on every affected merge: stuck worktrees accumulate, branches pile up, and stale tmux windows clutter the session — exactly the manual cleanup `run merge` is supposed to eliminate. Because the success envelope says `merged: true` + `supervisor.state: terminal`, the caller believes teardown happened and does not check.

## Hypotheses to investigate
- The supervisor submits the terminal report and exits **before** running teardown, or teardown runs but errors are swallowed (no surfaced warning in the merge envelope).
- Possible interaction with an interactive (`💻`/`stint`) worktree whose tmux window hosts the caller — the supervisor may skip/kill-window differently for a foreground interactive window vs a headless one, and may bail out of the whole teardown when the window kill can't proceed.
- Teardown may be racing the supervisor's own exit.

## Suggested fixes
- Make teardown run **before** the supervisor marks itself terminal / exits, and make it idempotent.
- If any teardown step fails, surface it in the `run merge` envelope (e.g. a `warnings[]` entry: "branch/worktree/window not reclaimed") instead of reporting a clean terminal.
- Add a `run gc` / reconcile command to reclaim orphaned worktrees+branches+windows for runs already terminal.
