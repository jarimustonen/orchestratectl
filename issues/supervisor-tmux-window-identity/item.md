---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
related: ['@supervisor-process-review-followup']
labels: [review-spinoff, supervisor]
---

# supervisor: qualify tmux liveness by session:window_id + socket

## Description

From supervisor-process /llm-review (F16). watchdog::tmux_window_present matches a bare #{window_name} via 'tmux list-windows -a' on the default socket. Window names are not unique across sessions, and multiple tmux servers/sockets are invisible — yielding false-positive ('agent alive' when it's a different session's window) and false-negative ('agent dead' on a non-default socket) liveness verdicts. Fix: record a fully-qualified tmux identity (session:window_id) plus the socket path at spawn time and match on that. Belongs with create.sh structured-stdout integration (all-kinds-spawn), which is where the qualified identity becomes available.
