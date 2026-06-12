---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# Spin-off proposal CLI (list/approve/reject)

## Description

orchestratectl spinoff list|approve|reject (domain verbs, §2.0). approve optionally calls issuectl new to materialize an issue. Implements --dry-run. **Depends on** state-schema-crate. **Validation gate**: V10 (issuectl --add-commit linkage; only blocks auto-materialization path).
