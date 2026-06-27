---
created: 2026-06-27
updated: 2026-06-28
type: task
reporter: jari
status: done
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, help-json]
closed: 2026-06-28
commits:
- hash: c550387
  summary: richer arg metadata + schema v2
- hash: c35207a
  summary: handoff note + requires-edges follow-up
---

# Expand --help --json arg metadata: flag aliases, constraints, arity, custom-parser values

## Description

v1 of the structured-help payload (schema_version_help=1) is a lossy projection of clap. v2 (bump schema_version_help) should add: flag long_aliases/short_aliases; mutual-exclusion/requirement edges (conflicts_with, requires, required-groups, required_if — clap getters permitting); global flag marker (is_global_set); value arity split (repeated vs multi-value, min/max num_args, value_delimiter, require_equals); help_heading; positional env + defaults (currently only flags carry them); accepted_values for custom value_parsers such as --output (jsonl|json|text + .json/.jsonl path patterns) which currently reports []. Audit each clap getter against AGENTS-AI-FIRST-CLI §14 and decide in/out explicitly. Also reconsider short-only flags (currently skipped per issue default) with a name=id fallback. Surfaced in issues/help-json-structured/review.md.
