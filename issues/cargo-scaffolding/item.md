---
created: 2026-06-12
updated: 2026-06-12
type: chore
assignee: jari
status: open
priority: high
epic: orchestratectl-mvp
---

# Cargo scaffolding for orchestratectl workspace

## Description

Workspace Cargo.toml, two crates (octl-core, octl-cli), CI-friendly defaults, AI-first CLI plumbing: --json everywhere with schema_version: 1, JSONL log subscriber via tracing, stderr error envelope ({schema_version, error: {code, message, invalid_value?, expected?}}), warnings array on stdout JSON payloads, exit codes 0/1/2 per §2 of AGENTS-AI-FIRST-CLI.md. No octl-tui crate (TUI deferred). See issues/orchestratectl-mvp/design.md and breakdown.md (row 1) for full context. No validation blockers.
