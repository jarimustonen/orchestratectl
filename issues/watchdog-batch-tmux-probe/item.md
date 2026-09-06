---
created: 2026-06-28
updated: 2026-06-28
type: improvement
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-28
commits:
- hash: e583d2a
  summary: 'perf(watchdog): batch tmux probe per socket + bound it with a timeout'
---

## Description

From /llm-review of @supervisor-tmux-window-identity. The liveness watchdog spawns one `tmux` subprocess per tracked node per tick (`watchdog::probe_window_qualified` / `probe_window_by_name`). At ~100 supervised agents this is ~100 forks per tick. Pre-existing in the legacy name-based path; the qualified path keeps the same per-node shape.

## Proposed fix

Group probes by socket and run one query per socket per tick: `tmux -S <socket> list-windows -a -F '#{window_id}'`. Build an in-memory set of live `window_id`s per socket, then evaluate every node against the snapshot. Preserves the tri-state semantics (socket unreachable -> Unknown for all nodes on it).

## Comments

Not a regression — split out of the qualified-identity change to keep that PR focused. Measure first (design.md §9 tracks supervisor process-count / poll-cost validation).
