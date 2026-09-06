---
created: 2026-06-27
updated: 2026-06-29
type: task
reporter: jari
status: fixed
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff, help-json]
closed: 2026-06-29
---

# Bound recursion depth of top-level --help --json (avoid whole-tree firehose)

## Description

Top-level 'taskfleet --help --output json' recursively serializes the entire command tree (~2100-line snapshot). AGENTS-AI-FIRST-CLI §14 says top-level help should NOT dump every flag of every subcommand; agents asking 'what commands exist?' pay for the whole tree. Decide+implement a shape: e.g. default depth 1 (each node lists immediate subcommands as name+about+aliases only, drill down per node for detail) with explicit opt-in for the full recursive tree. Contradicts the v1 shape help-json-structured deliberately shipped → schema_version_help bump + product decision, not a bugfix. Surfaced 4/4 in issues/help-json-structured/review.md.
