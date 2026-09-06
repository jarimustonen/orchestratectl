---
created: 2026-06-12
updated: 2026-06-29
type: feature
status: fixed
priority: normal
closed: 2026-06-29
---

# Always emit warnings: [] in success envelope (no elision)

## Description

The success envelope serializer (crates/taskfleet-cli/src/output.rs) currently skips the warnings field when the array is empty (#[serde(skip_serializing_if = ...)]). AGENTS-AI-FIRST-CLI §10 says warnings live in a warnings: [] array inside the stdout JSON; making agents branch on missing-vs-empty is a consumer tax. Change emit_json to always include warnings (empty array when no warnings), and update the version-subcommand integration tests to pin warnings == [] on the envelope. Surfaced by review of #version-subcommand — see history/review-version-subcommand.md §5. Depends on: cargo-scaffolding only.
