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

# supervisor: tolerate a corrupt JSONL line instead of looping forever

## Description

From supervisor-process /llm-review (F17). EventTail::poll (supervise/tail.rs) returns a hard corrupt_event_log error on a malformed line; the supervisor main loop logs a warning and continues, but next tick re-reads the SAME offset and fails identically — an infinite warn-spam loop where that tail never progresses and burns CPU. For a 'filesystem is the wire' design this is a real production hazard. Fix (needs an error-semantics decision): skip/advance past the bad line, OR quarantine to events.jsonl.corrupt-<ts>, OR halt only that one tail — and emit a one-shot supervisor.event_log_skipped_line event. Touches the core corrupt_event_log contract, so design it deliberately rather than inline.
