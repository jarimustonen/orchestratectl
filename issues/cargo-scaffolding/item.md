---
created: 2026-06-12
updated: 2026-06-12
type: chore
assignee: jari
status: done
priority: high
epic: orchestratectl-mvp
commits:
- hash: a4238157a7b0d3ca70c652952a3787a7fc9d81b5
  summary: 'docs(cargo-scaffolding): handoff notes + 6 review-spinoff issues'
- hash: 09e9874d505bf75fc01e96a620bff7c8b46f0862
  summary: 'fix(scaffolding): apply multi-LLM review findings'
- hash: 464d26abf9d3f2acd8bd32effdb482e2a79c2cad
  summary: 'feat(scaffolding): create octl-core + octl-cli workspace with AI-first plumbing'
closed: 2026-06-12
---

# Cargo scaffolding for orchestratectl workspace

## Description

Workspace Cargo.toml, two crates (octl-core, octl-cli), CI-friendly defaults, AI-first CLI plumbing: --json everywhere with schema_version: 1, JSONL log subscriber via tracing, stderr error envelope ({schema_version, error: {code, message, invalid_value?, expected?}}), warnings array on stdout JSON payloads, exit codes 0/1/2 per §2 of AGENTS-AI-FIRST-CLI.md. No octl-tui crate (TUI deferred). See issues/orchestratectl-mvp/design.md and breakdown.md (row 1) for full context. No validation blockers.
