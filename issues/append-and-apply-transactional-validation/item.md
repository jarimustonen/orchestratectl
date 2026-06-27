---
created: 2026-06-27
updated: 2026-06-27
type: improvement
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-27
---

# append_and_apply: validate before the durable append (avoid log poisoning)

## Description

Spun off from reducer-state-machine-hardening /llm-review (GPT-5.5 #5,#20).

`octl_core::events::append_and_apply_unlocked` writes the event line to events.jsonl and `sync_all()`s it **before** calling `apply_event`. If the reducer rejects the event (any `CorruptEventLog`: the new node.report XOR check, `require_status`, `discussion.resolved` missing resolution, the cross-run guard, etc.), the offending line is already durable in the append-only log. Every subsequent full replay / `rebuild_projections_from_events` then fails on that line, and a later append writes after the bad line, compounding corruption.

This is pre-existing and affects all reducer-side validation, not just the reducer-hardening change — but that change widened the set of events the reducer can reject, so it's worth fixing now.

Options:
1. Split reducer validation from application: a `validate_event(paths, &ev)` that runs (under the lock) before `append_event_with_seq`, so a rejected event is never appended.
2. Stage the append (temp/offset) and only commit (`set_len`/rename) after `apply_event` succeeds.

Tie-in: the new reducer tests (bare_report_payload_is_corrupt, etc.) currently leave a poison line in the test log and only assert the projection — once this lands, add an assertion that a rejected event is absent from events.jsonl. Coordinate with torn-write-truncate-tail / recover-last-seq work.

## Comments

### 2026-06-27T19:57:31Z · @jari

Closed by event-log-durability-trio: unified physical-line reader (read_all_events + find_prior_with_key share one torn-tail policy; recover_last_seq aligned on last-line + blank-line handling), append-side truncate_torn_tail before write, reducer::validate_event run before the durable append, and supervisor EventTail corrupt-line skip + one-shot supervisor.event_log_skipped_line.
