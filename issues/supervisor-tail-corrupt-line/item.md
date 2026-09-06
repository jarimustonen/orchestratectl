---
created: 2026-06-27
updated: 2026-06-27
type: improvement
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
related: ['@supervisor-process-review-followup']
labels: [review-spinoff, supervisor]
closed: 2026-06-27
---

# supervisor: tolerate a corrupt JSONL line instead of looping forever

## Description

From supervisor-process /llm-review (F17). EventTail::poll (supervise/tail.rs) returns a hard corrupt_event_log error on a malformed line; the supervisor main loop logs a warning and continues, but next tick re-reads the SAME offset and fails identically — an infinite warn-spam loop where that tail never progresses and burns CPU. For a 'filesystem is the wire' design this is a real production hazard. Fix (needs an error-semantics decision): skip/advance past the bad line, OR quarantine to events.jsonl.corrupt-<ts>, OR halt only that one tail — and emit a one-shot supervisor.event_log_skipped_line event. Touches the core corrupt_event_log contract, so design it deliberately rather than inline.

## Comments

### 2026-06-27T19:57:32Z · @jari

Closed by event-log-durability-trio: unified physical-line reader (read_all_events + find_prior_with_key share one torn-tail policy; recover_last_seq aligned on last-line + blank-line handling), append-side truncate_torn_tail before write, reducer::validate_event run before the durable append, and supervisor EventTail corrupt-line skip + one-shot supervisor.event_log_skipped_line.
