---
created: 2026-06-28
updated: 2026-06-28
type: improvement
reporter: jari
status: done
priority: normal
epic: taskfleet-mvp
related: ['@projection-key-body-consistency']
closed: 2026-06-28
commits:
- hash: 088d63f
  summary: 'refactor(core): typed node id + reducer plan-then-commit + corrupt-state error class'
---

# taskfleet-cli: state-file JSON/schema errors map to retryable io_error

## Description

Spin-off from projection-key-body-consistency /llm-review (gpt-5.5 #8, deepseek). from_core (crates/taskfleet-cli/src/run/mod.rs) maps Error::Json and Error::UnsupportedSchemaVersion to the generic system io_error (exit 2). For on-disk state files these are non-retryable data-integrity faults, not transient I/O — an AI caller's retry loop will hammer a file that will never parse. CorruptEventLog and (now) CorruptProjection already get distinct exit-1 user codes; malformed state JSON and unsupported state schema should too (e.g. corrupt-state-json, unsupported-state-schema). Also consider: write_* helpers don't validate schema_version before persisting (a future bug could write STATE_SCHEMA_VERSION+1, failing only on the next read). Decide whether writes should assert == STATE_SCHEMA_VERSION.

## Decisions

### 2026-06-28T02:21:55Z · @jari

Implemented in core-typing-pack pack (commit 088d63f). Decisions vs spec: (1) Unified ALL corrupt-persisted-state faults under one non-retryable 'corrupt_state' user code (exit 1) per the P8a task objective, rather than the distinct 'corrupt-state-json'/'unsupported-state-schema' codes the body sketched as 'e.g.'; this also re-homed the existing CorruptEventLog/CorruptProjection codes under 'corrupt_state'. (2) Delivered the body's binding fix: Error::Json/JsonBare and UnsupportedSchemaVersion now map to corrupt_state (exit 1) instead of retryable io_error (exit 2). (3) DEFERRED the objective's structured CorruptStateContext payload: its fields model event-log corruption and do not fit CorruptProjection's expected_id/body_id shape; threading a new field through ~49 sites + their reason assertions is disproportionate to 'small cleanups'. Enrichment still surfaced via envelope invalid_value/expected. Spin-off candidate. (4) Skipped optional write-side schema_version assertion (reducer always stamps STATE_SCHEMA_VERSION).
