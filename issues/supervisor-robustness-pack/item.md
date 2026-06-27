---
created: 2026-06-27
updated: 2026-06-27
type: chore
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Supervisor robustness pack: atomic PID claim + detach-reap + watchdog lock

## Description

Closes supervisor-pid-claim-race, supervisor-child-detach-reap, supervisor-watchdog-lock-retry in one coherent supervise/ change.
