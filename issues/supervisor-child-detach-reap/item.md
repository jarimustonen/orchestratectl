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
closed: 2026-06-28
---

# supervisor: detach + reap spawned child supervisors (zombies, SIGHUP)

## Description

From supervisor-process /llm-review (F14). spawn_child_supervisor (mod.rs) and reattach.rs drop the std::process::Child handle without reaping, so exited child supervisors become zombies (and kill(pid,0) reports zombies as alive, corrupting PID-staleness checks). Additionally the spawned processes stay in the caller's process group, so closing the terminal SIGHUPs all supervisors. Fix: double-fork / setsid pre_exec to fully detach long-lived supervisors, plus a periodic try_wait reaper (or store Child handles). Own design because it's a process-lifecycle/daemonization change touching both spawn paths.

## Add: cascade self-terminate to children (from test-harness-leaks-supervisors review, 2026-06-27)

Multi-model review of `test-harness-leaks-supervisors` raised (Gemini, Opus, DeepSeek consensus): when a parent supervisor self-terminates because its OWN run dir vanished (the new orphan defense in `supervise/mod.rs`), it does NOT signal its tracked `state.spawned_children`. Today this is self-healing in the common case — every child's run dir lives under the same root and vanishes simultaneously, so each child self-terminates within ~3s independently. But a child blocked on a lock / mid-`CHILD_DIR_WAIT` could outlive the parent. When this issue implements detach+reap, also have the run-dir-vanished shutdown path `SIGTERM` each known child PID before exiting, for a decisive whole-tree shutdown rather than relying on each level's independent self-terminate.

## Closure

Closed by **supervisor-robustness-pack** (branch `supervisor-robustness-pack`),
which fixed this together with the other two supervisor robustness issues in a
single coherent `supervise/` change. See the wrapper issue and
`issues/supervisor-robustness-pack/handoff.md` for the combined change,
multi-model review fixes, and deferred follow-ups.
