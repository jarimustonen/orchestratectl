---
created: 2026-06-27
updated: 2026-06-27
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Event-log durability: unified reader + validate-before-append + tail recovery

## Description

Closes torn-line-policy-consistency, append-and-apply-transactional-validation, and supervisor-tail-corrupt-line in one coherent pass. Defines the canonical physical-line-reader and write-staging discipline.
