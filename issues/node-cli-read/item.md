---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
commits:
- hash: cfdd513
  summary: 'feat(node-cli-read): node list/show/report subcommands'
---

# Node CLI (list/show/report)

## Description

orchestratectl node list|show|report. report is a domain verb (§2.0 of design.md); ingests a JSON file matching the §7.3 payload spec and emits node.report event under flock. Implements --dry-run and --idempotency-key (defends against agent retry storms). **Depends on** state-schema-crate.
