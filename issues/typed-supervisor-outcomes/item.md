---
created: 2026-08-15
updated: 2026-08-15
type: task
status: open
priority: high
epic: lifecycle-architecture-review
---

# Thin supervisor: encode terminal outcomes as a typed table

## Description

## Goal
Replace supervisor terminal-state inference with the typed outcome table from `issues/lifecycle-architecture-review/design.md` §2.6 (A6).

## Context
`run merge` is the only success truth, but not the only terminal truth. Model the negative outcomes explicitly:

- explicit merge → done + teardown;
- non-zero/signal exit-status event → failed, preserve branch/worktree;
- exit 0 with no merge → attention-required, non-terminal;
- run cancel / per-node cancel → cancelled, preserve branch/worktree;
- confirmed-death backstop → failed, preserve branch/worktree;
- blocked node report → blocked/manual, preserve branch/worktree.

This should delete or bypass pid×pane×branch×report heuristics as primary state. Keep pid liveness only as the residual crash backstop with a short persisted post-death grace and an exclusive-lock reread before failing.

## Done criteria
- Outcome transitions are explicit and table-driven enough that tests encode the table.
- Cancel and blocked outcomes never authorize teardown of unmerged work.
- Old idle/activity-clock success inference is removed or no longer reachable.
- Full project green gate passes, including docs.
