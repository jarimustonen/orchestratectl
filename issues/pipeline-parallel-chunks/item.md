---
created: 2026-07-25
updated: 2026-08-05
type: task
status: fixed
priority: normal
related: ['@pipeline-walking-skeleton']
commits:
- hash: 4d64265
  summary: concurrent DAG-wave chunk scheduling with deterministic merge + floor re-check + rebase-and-fix
- hash: 7d9d818
  summary: review fixes — invariant-5 preserve remaining wave builds on mid-merge early return
closed: 2026-08-05
---

# Pipeline: parallel independent chunks (DAG scheduling)

## Description

The T5 skeleton runs chunks strictly sequentially in a topological order, each stacking on the moved feat/<slug> tip. Schedule independent chunks (no dep path between them) concurrently in separate worktrees, then merge them in a deterministic order with the floor re-checked at each merge (design §6 VAIHE 2). Handle merge-conflict between concurrently-built chunks via the deterministic rebase-and-fix protocol. Bounded by the macOS PTY / process-count limits (see pipeline-circuit-breakers).
