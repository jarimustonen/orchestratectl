---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
related: ['@split-stint-start-handoff']
---

# Companion skill resources are claude-only; codex flat layout unsupported

## Description

Bundled-skill companion resources (introduced with split-stint-start-handoff) install only for the claude agent, because the codex layout is a flat `~/.codex/prompts/` dir where a per-skill sibling would be un-namespaced (collision risk across skills) and cross-skill links like `../stint-start/AGENTS-EXECUTION-DAG.md` cannot resolve. Consequence: a codex install of stint-start/stint-handoff has an unresolvable in-body reference to the shared DAG file. If codex becomes a first-class target for these skills, decide the layout: namespaced companion filenames (`stint-start--AGENTS-EXECUTION-DAG.md`) with agent-specific link rendering, or a shared `~/.codex/prompts/_shared/` dir. Deferred: the user's primary agent is claude; codex is a secondary export.
