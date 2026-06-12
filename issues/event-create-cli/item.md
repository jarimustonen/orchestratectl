---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Event create CLI (sanctioned write path)

## Description

orchestratectl event create --kind --node-id --from-file [--idempotency-key] — sanctioned write path for skill-shim and external bash tools. Must run the reducer to update affected projection files (manifest.json, nodes/*.json, etc.) atomically within the same flock window (per design.md §2.3). Validates kind against the known event-kind set; unknown rejected. Implements --dry-run. **Depends on** state-schema-crate.
