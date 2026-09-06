---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: wontfix
priority: normal
epic: taskfleet-mvp
closed: 2026-06-29
---

# RunWriter: cached next_seq + batched fsync

## Description

Spin-off from state-schema-crate review (gpt-5.5 #1, #3, #18, #20). Replace the per-event recover_last_seq + fsync path with a long-lived RunWriter guard that holds the per-run flock, caches next_seq in memory, and batches fsync at configurable durability. Goal: get short-lived appends below the V4 latency budget (<10 ms p99) and give supervisors a clean ergonomic API. Includes hiding append_event_with_seq behind the guard.
