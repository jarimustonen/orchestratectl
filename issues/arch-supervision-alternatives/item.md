---
created: 2026-08-12
updated: 2026-08-12
type: task
status: done
priority: normal
epic: lifecycle-architecture-review
deliverable: issues/lifecycle-architecture-review/alternatives.md
closed: 2026-08-12
---

# Survey alternative supervision architectures

## Description

PHASE 1 (parallel). Given the real requirements — spawn N external agent processes in git worktrees, know reliably when each is DONE, merge, tear down, survive agent/supervisor crashes — survey supervision architectures beyond today's polling-watchdog: protocol/state-machine where the worker self-reports transitions over a reliable channel; exit-code + named-pipe/FIFO completion signaling; event-sourced with a worker heartbeat/lease; a thin 'the worker calls run merge and THAT is the only completion truth' model. Compare each vs the current design on edge-case surface, crash-recovery, and complexity. Deliverable: issues/lifecycle-architecture-review/alternatives.md. Research only.
