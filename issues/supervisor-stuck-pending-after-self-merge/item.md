---
created: 2026-07-16
updated: 2026-07-16
type: bug
reporter: jari
status: open
priority: high
---

# Spinoff supervisor stuck at status=pending (no teardown) despite successful self-merge — 5/9 in a headless batch

_Source: supervisor merge→report→teardown handoff_

## Description

See analysis.md — supervisor never records node.report / never tears down though the agent self-merged (git-verified). Intermittent under high fan-out (5/9 headless spinoffs stuck ~21.7h with live supervisor + tmux window; run status reports false pending).
