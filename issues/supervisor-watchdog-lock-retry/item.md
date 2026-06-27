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

# supervisor: watchdog last_report read under lock + synthesis dedup

## Description

From supervisor-process /llm-review (F15). watchdog_tick (mod.rs ~537-559) reads n.last_report.is_none() OUTSIDE the run lock, then calls append_and_apply to synthesize a terminal node.report. A real report arriving in that window can produce a duplicate terminal node.report (idempotent on the parent side via deterministic dedup + last-writer-wins, so no wrong outcome — just two events). Fix: acquire the run lock and re-read last_report under it before synthesizing, or use a conditional append. Best designed together with the watchdog half-state retry state machine (the FIX for F4 in review-followup) so watchdog locking + retry are coherent.

## Closure

Closed by **supervisor-robustness-pack** (branch `supervisor-robustness-pack`),
which fixed this together with the other two supervisor robustness issues in a
single coherent `supervise/` change. See the wrapper issue and
`issues/supervisor-robustness-pack/handoff.md` for the combined change,
multi-model review fixes, and deferred follow-ups.
