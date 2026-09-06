---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
related: ['@supervise-gate-test-flake']
labels: [test, review-spinoff]
closed: 2026-06-28
commits:
- hash: 1dea693
  summary: lenient JSONL poll
---

# supervise_gates: tolerate partial trailing JSONL line during readiness polling

## Description

Spin-off from supervise-gate-test-flake review (poll_until). `read_events`/`count_kind` in crates/taskfleet-cli/tests/supervise_gates.rs `unwrap()` every JSONL line. `wait_for_kind` polls events.jsonl while a detached supervisor may still be appending; if a poll observes a half-written trailing line, `serde_json::from_str(l).unwrap()` panics, converting a transient state into a test failure. Add a tolerant counter (`filter_map(... .ok())`) used only inside readiness polling, keeping strict parsing for final assertions. Pre-existing; flagged by GPT-5.5 but out of scope for the pure de-flake.
