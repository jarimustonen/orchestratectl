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

# supervisor: watchdog last_report read under lock + synthesis dedup

## Description

From supervisor-process /llm-review (F15). watchdog_tick (mod.rs ~537-559) reads n.last_report.is_none() OUTSIDE the run lock, then calls append_and_apply to synthesize a terminal node.report. A real report arriving in that window can produce a duplicate terminal node.report (idempotent on the parent side via deterministic dedup + last-writer-wins, so no wrong outcome — just two events). Fix: acquire the run lock and re-read last_report under it before synthesizing, or use a conditional append. Best designed together with the watchdog half-state retry state machine (the FIX for F4 in review-followup) so watchdog locking + retry are coherent.
