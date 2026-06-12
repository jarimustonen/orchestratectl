---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Discussion CLI (list/show/resolve)

## Description

orchestratectl discussion list|show|resolve (resolve is a domain verb, §2.0). Mutation writes discussion.resolved event, updates JSON under flock. Implements --dry-run. **Depends on** state-schema-crate.
