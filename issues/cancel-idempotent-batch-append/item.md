---
created: 2026-06-28
updated: 2026-06-28
type: improvement
status: open
priority: normal
epic: orchestratectl-mvp
---

# run cancel: idempotency keys + batch append for synthesized cancel events

## Description

Spun off from run-cancel-terminal-run-semantics /llm-review (gpt-5.5, opus).

core::cancel_run synthesizes one node.report per live node plus one run.status via append_and_apply_unlocked, each with idempotency_key: None and its own fsync. Two related gaps:

1. **Crash-retry duplicates (correctness-ish):** if the process crashes after a node.report is appended+fsynced but before apply_event writes the projection, a re-cancel reads the node as live and appends a SECOND node.report. Reducer terminal-guard keeps projections correct, but the event log gains duplicate logical-cancel events (auditors, metrics, future rebuild see them). Fix: deterministic idempotency keys, e.g. run-cancel:<run_id>:node:<node_id> and run-cancel:<run_id>:run-status, via a lock-held dedup helper (find_prior_with_key is already pub(crate)).

2. **Perf / lock-hold (operational):** each append re-runs truncate_torn_tail + recover_last_seq + validate + fsync. For a 1000-node fan-out cancel that is N opens + N fsyncs under one held lock, blocking supervisors/report ingestion for seconds. Fix: a batch append primitive (append_and_apply_batch_unlocked) that recovers seq once, validates all up front, writes all lines, fsyncs once, then applies projections.

Out of scope for the parent issue (which fixed terminal-run refusal + convergent re-cancel + the single-lock honesty guarantee). Both gaps pre-date this change but are magnified by holding one lock for the whole transaction.
