---
created: 2026-06-27
updated: 2026-06-27
type: improvement
reporter: jari
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-27
---

# Align torn-line policy across read_all_events and the append/truncate path

## Description

Multi-model review of idempotency-lookup-into-core surfaced two torn-line consistency gaps OUTSIDE that issue's read-side scope. (1) read_all_events (events.rs) uses BufReader::lines(), which silently accepts a valid-JSON final line lacking a trailing newline — a line recover_last_seq and find_prior_with_key both now discard as an uncommitted partial write. A reducer replay via read_all_events would therefore apply an event recovery considers unwritten, and the next append would reuse its seq. (2) append_and_apply_unlocked/write_event_line open events.jsonl in append mode and write at EOF without truncating a torn (newline-less) tail first; recover_last_seq only IGNORES the tail for seq purposes, so the next append concatenates onto the partial bytes, producing a newline-terminated malformed line that later hard-errors. Fix direction: extract a single physical-line reader with an explicit torn-tail policy and build recover_last_seq, read_all_events, and find_prior_with_key on it; have the append path truncate to the last complete record under the lock before writing. Source: history/review-idempotency-lookup-into-core.md (Gemini #3, GPT #2).

## Comments

### 2026-06-27T19:57:31Z · @jari

Closed by event-log-durability-trio: unified physical-line reader (read_all_events + find_prior_with_key share one torn-tail policy; recover_last_seq aligned on last-line + blank-line handling), append-side truncate_torn_tail before write, reducer::validate_event run before the durable append, and supervisor EventTail corrupt-line skip + one-shot supervisor.event_log_skipped_line.
