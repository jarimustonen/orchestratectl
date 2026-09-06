---
created: 2026-08-12
updated: 2026-08-17
type: bug
status: fixed
priority: normal
labels: [observability, keep-0.2]
lane: lifecycle
lane_seq: 60
closed: 2026-08-17
---

# run show reports null worktree_path/source_branch for a live pending run

## Description

## Observed
For a live `--kind spinoff` run in `pending` state whose worktree and tmux window demonstrably exist, `taskfleet run show <id> --output json` returns:
```
worktree_path: null
source_branch: null
```
Verified the worktree really exists via `git worktree list` (in the target repo) and the run's tmux-pane cwd (`tmux display-message -p -t <window> '#{pane_current_path}'`), which pointed at `<repo>__worktrees/wt-<short>-<title>`.

## Impact
Because both fields are null, `run show` alone gives no way to tell **which repo** a run operates in. In a multi-repo session (cross-repo campaign spawning same-titled runs per binary) this is genuinely confusing — I twice mistook an `taskfleet`-repo run for the implementer of an issue in a *different* repo, because the run title matched and `run show` did not disclose the repo/worktree. Had to fall back to `git worktree list` + pane cwd.

## Expected
Once a run's worktree is materialized, `run show` (and `run list`) should surface `worktree_path` and `source_branch` (hence the repo), even while status is `pending`. If they are intentionally deferred until some later transition, document when they populate.

## Repro
Spawn any `--kind spinoff`, then `taskfleet run show <id> --output json` while it is still `pending` with a live worktree; observe the null fields.

## Env
taskfleet 0.1.5.

## Decisions

### 2026-08-13T11:10:42Z · @adr-decision-2

KEEP-and-fix: A5 REQUIRES run show to expose the worktree path for attention-required runs — the fix becomes mandatory. Surface survives the thin model; fix is model-independent. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).

## Resolution

### 2026-08-17T19:01:47Z · @issuectl

Implemented pending-state worktree/source coordinates on run show and run list, with replay-safe projection updates and integration coverage.
