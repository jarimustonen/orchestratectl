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

# supervisor: detach + reap spawned child supervisors (zombies, SIGHUP)

## Description

From supervisor-process /llm-review (F14). spawn_child_supervisor (mod.rs) and reattach.rs drop the std::process::Child handle without reaping, so exited child supervisors become zombies (and kill(pid,0) reports zombies as alive, corrupting PID-staleness checks). Additionally the spawned processes stay in the caller's process group, so closing the terminal SIGHUPs all supervisors. Fix: double-fork / setsid pre_exec to fully detach long-lived supervisors, plus a periodic try_wait reaper (or store Child handles). Own design because it's a process-lifecycle/daemonization change touching both spawn paths.
