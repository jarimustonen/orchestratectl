---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
commits:
- hash: 0cb3c93 feat(version)
  summary: finalize version subcommand contract
---

# version subcommand

## Description

orchestratectl version [--json] returning {schema_version, version, commit, state_schema_version, supported_state_schemas}. Per AGENTS-AI-FIRST-CLI §10 — agents need to detect drift between trained expectations and actual binary. Cheap; can land any time after scaffolding. **Depends on** cargo-scaffolding only.
