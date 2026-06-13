---
created: 2026-06-12
updated: 2026-06-13
type: task
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
commits:
- hash: c55487d
  summary: 'feat(output): replace --json with --output text|json|jsonl (default jsonl)'
- hash: '5793917'
  summary: 'test(output): migrate suite to --output flag, add jsonl single-line + legacy-flag tests'
- hash: a18c88a
  summary: 'docs(skills): update SKILL.md seeds for --output flag (default jsonl)'
---

# Global --output flag and --output=jsonl streaming

## Description

Implement the global --output flag per AGENTS-AI-FIRST-CLI §9/§12/§13: --output=text|json|jsonl format selector, --output FILE write-to-file alternative, and the JSONL streaming envelope (schema_version, event, seq) with terminal result/cancelled/error events. Required before long-running subcommands (event tail --follow, supervise) can ship. Surfaced by cargo-scaffolding multi-LLM review.
