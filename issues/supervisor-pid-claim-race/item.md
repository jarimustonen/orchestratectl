---
created: 2026-06-27
updated: 2026-06-27
type: improvement
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
related: ['@supervisor-process-review-followup']
labels: [review-spinoff, supervisor]
---

# supervisor: atomic PID-file ownership claim (TOCTOU race)

## Description

From supervisor-process /llm-review (F7). The startup PID check at supervise/mod.rs:81-102 reads a stale supervisor.pid, then writes its own, with no lock between — two concurrent 'supervise' or 'run reattach' invocations can both claim the same run and both enter the main loop, violating the one-supervisor-per-run invariant. Same race in run/reattach.rs. Fix: claim ownership atomically under the run flock (or O_EXCL/flock on the pid file itself) in BOTH supervise startup and reattach; migrate legacy plain-integer pid files. Consciously deferred in issues/supervisor-process/handoff.md ('revisit if multi-launch races appear in V5/V6'); filing as the tracked hardening issue. Pairs with the start-time identity work (handled separately as a FIX in the review-followup branch).
