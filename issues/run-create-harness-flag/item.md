---
created: 2026-08-10
updated: 2026-08-11
type: feature
status: in-progress
priority: normal
labels: [from-homebase-research]
---

# run create --harness: promote the pi adapter (and others) from bakeoff into real runs

## Description

## Origin
Recommendation from a homebase research report (`pidev-harness-migration`, 2026-08-09): Jari
wants to migrate worktree/agent work from Claude Code to the **pi.dev harness** (Mario
Zechner / Earendil, MIT — AGENTS.md-native, Agent-Skills standard, print/JSON/RPC/SDK
headless).

## Key local finding
orchestratectl **already ships a `pi` harness adapter** — `harness bakeoff` lists adapters
`aider, claude, claude-deepseek, pi` behind a `CodeHarness` seam. But that seam is **not wired
into `run create`**: every real run (spinoff/orchestrate/fan-out/research/code) still
hard-launches `claude` in a tmux pane. So this is mostly **promotion of an existing adapter**,
not building a harness from scratch.

## What this feature does
1. **`run create --harness <name>`** (default `claude`) — routes the worker launch through the
   selected `CodeHarness` adapter for all run kinds, instead of the hardcoded claude launch.
2. **Per-kind default + config precedence** (§ AGENTS-AI-FIRST-CLI §8): flag > env
   (`ORCHESTRATECTL_HARNESS`) > config file > built-in default (`claude`). So a repo/user can
   default `research`/`spinoff` to `pi` while keeping `claude` for interactive `code`.
3. **Skill/Agent-tool translation shim** where the harness lacks Claude-specific tools (pi has
   no sub-agents/MCP/permission-sandbox by design — mostly fine since orchestration is already
   external, but the Skill/Agent tools need mapping).
4. Surface the chosen harness in `run show`/`run list --json` + the event log.

## Suggested rollout (from the research)
Don't big-bang. Land `--harness`, then adopt pi for **one autonomous kind first**
(`research` or `spinoff`); Claude stays default + the interactive driver. `harness bakeoff`
already lets Jari compare pi vs claude on his own box before flipping a default.

## Acceptance
- `run create --harness pi` launches a pi-driven worker for at least the autonomous kinds,
  merges + reports through the same supervisor path as claude.
- Config precedence + `--json` surfacing; docs + companion skills updated; `version --json`
  unaffected or bumped per §10.

Full rationale + stack mapping: homebase `research/pidev-harness-migration.md`.
Target: next minor (**0.2.0**) — homebase has a follow-up gated on that release.
