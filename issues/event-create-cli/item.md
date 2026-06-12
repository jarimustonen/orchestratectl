---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
commits:
- hash: bac6e0f9fed42f6aa0ede725811d706c7bf0ceaa
  summary: 'feat(event-create-cli): sanctioned write path for events'
- hash: 75b676c81db16840defec8a47b81fd57bbe75184
  summary: 'fix(event-create-cli): apply llm-review findings'
closed: 2026-06-12
---

# Event create CLI (sanctioned write path)

## Description

orchestratectl event create --kind --node-id --from-file [--idempotency-key] — sanctioned write path for skill-shim and external bash tools. Must run the reducer to update affected projection files (manifest.json, nodes/*.json, etc.) atomically within the same flock window (per design.md §2.3). Validates kind against the known event-kind set; unknown rejected. Implements --dry-run. **Depends on** state-schema-crate.
