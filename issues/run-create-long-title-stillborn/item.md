---
created: 2026-08-14
updated: 2026-08-16
type: bug
status: open
priority: normal
labels: [architecture]
lane: lifecycle
lane_seq: 20
---

# run create with a long --title spawns stillborn (tmux window-name truncation mismatch)

## Description

`orchestratectl run create --kind spinoff --headless --title "<long title>"` deterministically produces a **stillborn** run (supervisor never records, `node_count: 0`, no reason persisted in the run dir) when the title is long enough that the derived worktree/branch name is truncated.

## Evidence (real, 2026-08-14)

Title `"DAG head-of-line: in-progress issues are resumable, not excluded (stint-head-of-line-in-progress-eligible)"` → branch `wt/01kzzbpcms-dag-head-of-line-in-progress-issues-are-` (note the truncation + trailing `-`). `create.sh` reports the worktree + window created OK, then the follow-up window lookup fails:

```
✓ Successfully created worktree and tmux window for 'wt/01kzzbpcms-dag-head-of-line-in-progress-issues-are-'
{ "error": { "code": "tmux-window-not-found",
  "message": "No tmux window for 'wt/01kzzbpcms-dag-head-of-line-in-progress-issues-are-' (or flat '...') in session 'headless'" } }
Cleaning up partial state (exit 1)...
```

The window is created under a tmux-truncated name, but the lookup searches for the full (untruncated) branch-derived name → no match → create.sh exits 1 → stillborn. Reproduced 3×; a **short** title spawned fine immediately.

## Impact

Silent-ish failure: `run create` returns a `create_sh_error` envelope, but if the caller redirects/ignores stdout the run just sits `pending`/stillborn with **no reason recorded in `~/.orchestratectl/runs/<id>/manifest.json`** (`error: null`). Wastes a spawn and, in an autonomous batch, looks like an unexplained stillborn (masquerades as the `supervisor-spawn-fails-silently-at-run-create` class).

## Fix direction

Make the branch/window name derivation and the subsequent window lookup use the **same** (truncated) name — derive a bounded-length slug for the worktree/window up front and look it up by that exact value; and/or cap the title→branch length deterministically. Also persist the create.sh failure reason onto the run so a stillborn is diagnosable from `run show` without re-running with captured stdout. Relates to `supervisor-spawn-fails-silently-at-run-create`.

