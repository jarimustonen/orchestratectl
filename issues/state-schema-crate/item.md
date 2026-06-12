---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: done
priority: high
epic: orchestratectl-mvp
commits:
- hash: at(octl-core)
  summary: state schema, flock, event append + reducer
- hash: 0835f37
  summary: 'fix(octl-core): apply multi-LLM review findings + handoff'
closed: 2026-06-12
---

# State schema crate (octl-core)

## Description

octl-core: schema types (manifest, node, event, discussion, spinoff), atomic write helpers, per-run flock, event append primitive + seq counter. Includes lifecycle: autonomous|interactive, parent_run_id/parent_node_id, and all 8 kinds in the enum. Snapshot tests against fixture runs. State files carry their own schema_version (starting at 1) separate from CLI output schema_version. **Depends on** cargo-scaffolding. **Validation gate**: V4 (fs2 flock APFS stress test, see validation.md).
