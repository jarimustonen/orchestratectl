---
created: 2026-08-11
updated: 2026-08-11
type: feature
status: open
priority: normal
labels: [from-homebase-research]
---

# pi worker bundled-SKILL prompt translation shim

## Description

Follow-up to run-create-harness-flag. run create --harness pi now launches a pi agent in the worker pane, but the bundled SKILL workflows the worker runs are Claude-Code-flavored (Skill/Agent tools, /worktree-merge slash commands, sub-agents/MCP — none of which pi has). pi is AGENTS.md-native with the Agent-Skills standard. Translate the worker-facing prompt/SKILL surface so a pi worker can actually complete the spinoff/research loop (work -> orchestratectl run merge -> report). Most orchestration is already external (run merge is a plain CLI call pi can run via bash), so the gap is narrow: map the Skill/Agent-tool references. Scope: make ONE autonomous kind (research or spinoff) complete end-to-end under pi.
