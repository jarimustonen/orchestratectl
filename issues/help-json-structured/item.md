---
created: 2026-06-12
updated: 2026-06-27
type: task
status: done
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff, cargo-scaffolding-review]
related: ['@help-json-clap-native-resolution', '@help-json-richer-arg-metadata', '@help-json-depth-control', '@help-json-deprecation-convention']
closed: 2026-06-27
commits:
- hash: f8c90ce
  summary: walker module
- hash: 911f51b
  summary: snapshot + contract tests
- hash: e3e35ed
  summary: review-driven hardening
- hash: 5070df5
  summary: assessment + spin-offs
---

# Structured --help --json output across all subcommands

## Description

Per AGENTS-AI-FIRST-CLI §14, every <tool> ... --help accepts --json and emits a structured description (subcommands, flags, args, defaults, env-var mappings, accepted-value enums, deprecation status, schema_version of the help payload). Clap derive does not ship this; needs a custom layer that walks clap::Command. Surfaced by cargo-scaffolding multi-LLM review.
