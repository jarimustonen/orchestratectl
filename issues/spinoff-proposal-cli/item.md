---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
commits:
- hash: ee3840aa7e743bcac51f220f243c229205022434
  summary: 'feat(spinoff-proposal-cli): spinoff list/approve/reject'
- hash: 55cb1e6ad85628c21678d5df4f74e1cbae05ca6d
  summary: 'fix(spinoff-proposal-cli): apply llm-review findings'
- hash: 21a48b801e100e6a2743f68d12d064a414a176ee
  summary: 'chore(spinoff-proposal-cli): file llm-review spin-off issues and discuss handoff'
closed: 2026-06-12
---

# Spin-off proposal CLI (list/approve/reject)

## Description

orchestratectl spinoff list|approve|reject (domain verbs, §2.0). approve optionally calls issuectl new to materialize an issue. Implements --dry-run. **Depends on** state-schema-crate. **Validation gate**: V10 (issuectl --add-commit linkage; only blocks auto-materialization path).
