---
created: 2026-06-28
updated: 2026-06-28
type: improvement
reporter: jari
status: open
priority: normal
epic: orchestratectl-mvp
related: ['@projection-key-body-consistency']
---

# octl-cli: state-file JSON/schema errors map to retryable io_error

## Description

Spin-off from projection-key-body-consistency /llm-review (gpt-5.5 #8, deepseek). from_core (crates/octl-cli/src/run/mod.rs) maps Error::Json and Error::UnsupportedSchemaVersion to the generic system io_error (exit 2). For on-disk state files these are non-retryable data-integrity faults, not transient I/O — an AI caller's retry loop will hammer a file that will never parse. CorruptEventLog and (now) CorruptProjection already get distinct exit-1 user codes; malformed state JSON and unsupported state schema should too (e.g. corrupt-state-json, unsupported-state-schema). Also consider: write_* helpers don't validate schema_version before persisting (a future bug could write STATE_SCHEMA_VERSION+1, failing only on the next read). Decide whether writes should assert == STATE_SCHEMA_VERSION.
