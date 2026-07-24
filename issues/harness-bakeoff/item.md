---
created: 2026-07-24
updated: 2026-07-24
type: task
status: fixed
priority: high
closed: 2026-07-24
---

# Harness bake-off: claude + claude-deepseek + pi CodeHarness adapters + bakeoff runner (run one brief through all 4 agent loops and compare)

## Description

## Agent Runs

### 2026-07-24T16:47:16Z · @agent-harness-bakeoff

Implemented harness::support (AgentLaunch + run_chunk skeleton extracted from aider), new adapters claude/claude-deepseek (claude.rs) and pi (pi.rs), and 'orchestratectl harness bakeoff' runner (bakeoff.rs) + CLI wiring. pi fully wired via npm @earendil-works/pi-coding-agent (no follow-up stub needed). 68 harness unit tests green; conformance stub/fixture-backed, live tests gated on OCTL_HARNESS_LIVE=1. /llm-review (4 models) run; 7 consensus findings fixed: usage-parser final-aggregate selection, transcript stream separator, seed_repo symlink/dir rejection, malformed-JSON brief error, empty-credential fast-fail, token-total overflow guard, claude argv double-flag tests. fmt+clippy clean; full workspace green.
