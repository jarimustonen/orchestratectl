---
created: 2026-06-12
updated: 2026-06-12
type: task
status: open
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
---

# Structured --help --json output across all subcommands

## Description

Per AGENTS-AI-FIRST-CLI §14, every <tool> ... --help accepts --json and emits a structured description (subcommands, flags, args, defaults, env-var mappings, accepted-value enums, deprecation status, schema_version of the help payload). Clap derive does not ship this; needs a custom layer that walks clap::Command. Surfaced by cargo-scaffolding multi-LLM review.
