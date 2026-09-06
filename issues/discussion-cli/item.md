---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
commits:
- hash: 65e5f43
  summary: 'feat(discussion-cli): list/show/resolve verbs'
- hash: c1832f1
  summary: 'test(discussion-cli): integration tests'
- hash: '40221e3'
  summary: 'fix(discussion-cli): apply multi-LLM review findings'
closed: 2026-06-12
---

# Discussion CLI (list/show/resolve)

## Description

taskfleet discussion list|show|resolve (resolve is a domain verb, §2.0). Mutation writes discussion.resolved event, updates JSON under flock. Implements --dry-run. **Depends on** state-schema-crate.
