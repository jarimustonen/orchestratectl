---
created: 2026-06-28
updated: 2026-06-28
type: chore
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-28
---

# Symlink containment: pid file + O_NOFOLLOW on projections

## Description

Closes supervisor-pid-symlink-containment + run-state-symlink-toctou-openat2 (latter with macOS+Linux O_NOFOLLOW, openat2 deferred).
