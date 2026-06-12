---
created: 2026-06-12
updated: 2026-06-12
type: task
status: open
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
---

# Global --output flag and --output=jsonl streaming

## Description

Implement the global --output flag per AGENTS-AI-FIRST-CLI §9/§12/§13: --output=text|json|jsonl format selector, --output FILE write-to-file alternative, and the JSONL streaming envelope (schema_version, event, seq) with terminal result/cancelled/error events. Required before long-running subcommands (event tail --follow, supervise) can ship. Surfaced by cargo-scaffolding multi-LLM review.
