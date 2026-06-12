---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
commits:
- hash: 65e5f43
  summary: 'feat(discussion-cli): list/show/resolve verbs'
- hash: c1832f1
  summary: 'test(discussion-cli): integration tests'
---

# Discussion CLI (list/show/resolve)

## Description

orchestratectl discussion list|show|resolve (resolve is a domain verb, §2.0). Mutation writes discussion.resolved event, updates JSON under flock. Implements --dry-run. **Depends on** state-schema-crate.
