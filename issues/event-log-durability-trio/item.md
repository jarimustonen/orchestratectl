---
created: 2026-06-27
updated: 2026-06-27
type: feature
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-27
---

# Event-log durability: unified reader + validate-before-append + tail recovery

## Description

Closes torn-line-policy-consistency, append-and-apply-transactional-validation, and supervisor-tail-corrupt-line in one coherent pass. Defines the canonical physical-line-reader and write-staging discipline.

## Comments

### 2026-06-27T19:57:41Z · @jari

Implemented chunks A–E (unified reader, truncate-before-write, validate-before-append, supervisor tail recovery, tests). Quality bar green (build/test/clippy/fmt). Multi-model /llm-review run; localized findings fixed (recover_last_seq alignment, shared node_id helper, tail buffer reuse + dedup hardening, doc honesty); architectural findings spun off as reducer-validate-apply-unify, corrupt-line-quarantine, idempotency-scan-strictness-and-index. Closes torn-line-policy-consistency, append-and-apply-transactional-validation, supervisor-tail-corrupt-line.
