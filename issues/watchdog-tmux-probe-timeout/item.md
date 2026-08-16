---
created: 2026-06-28
updated: 2026-06-28
type: improvement
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
commits:
- hash: e583d2a
  summary: 'perf(watchdog): batch tmux probe per socket + bound it with a timeout'
---

## Description

From /llm-review of @supervisor-tmux-window-identity. The watchdog's `Command::output()` calls on `tmux` have no timeout. A wedged tmux client (hung server, stuck socket) blocks the entire watchdog tick, stalling liveness for every supervised node.

## Proposed fix

Wrap the tmux probe in a bounded wait (spawn + wait-with-timeout, or a helper). On timeout, return `TmuxProbe::Unknown` (defer to PID liveness) rather than blocking or reaping. Applies to both `probe_window_qualified` and `probe_window_by_name`.

## Comments

Pre-existing; split out of the qualified-identity change. Compose with @watchdog-batch-tmux-probe (a per-socket batch query needs the same timeout).
