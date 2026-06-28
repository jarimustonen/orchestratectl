---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
related: ['@supervisor-process-review-followup']
labels: [review-spinoff, supervisor]
commits:
- hash: '3130460'
  summary: homebase create.sh emits qualified tmux identity (branch create-sh-tmux-identity)
- hash: 67c4233
  summary: 'core: TmuxIdentity + Node.tmux_identity + reducer folding'
- hash: 84a83eb
  summary: 'cli: watchdog qualified-identity match + spawn parse + back-compat warn'
- hash: e6c248c
  summary: 'docs: design.md §8.1/§1.3 qualified tmux identity'
- hash: 2ea0eca
  summary: homebase create.sh review hardening (socket scoped to window + window-id guard)
- hash: 32b5d0a
  summary: review hardening — tri-state probe + socket-scoped window_id + empty-string normalization
closed: 2026-06-28
---

# supervisor: qualify tmux liveness by session:window_id + socket

## Description

From supervisor-process /llm-review (F16). watchdog::tmux_window_present matches a bare #{window_name} via 'tmux list-windows -a' on the default socket. Window names are not unique across sessions, and multiple tmux servers/sockets are invisible — yielding false-positive ('agent alive' when it's a different session's window) and false-negative ('agent dead' on a non-default socket) liveness verdicts. Fix: record a fully-qualified tmux identity (session:window_id) plus the socket path at spawn time and match on that. Belongs with create.sh structured-stdout integration (all-kinds-spawn), which is where the qualified identity becomes available.
