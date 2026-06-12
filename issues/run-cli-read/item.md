---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: high
epic: orchestratectl-mvp
---

# Run CLI (read + lifecycle)

## Description

orchestratectl run create|list|show|cancel|reattach. create initializes a run dir; top-level vs child-spawn behavior per design.md §7.2 (top-level spawns supervisor; child-spawn writes child.spawned to parent and exits without spawning supervisor — parent supervisor does that from its tail-follow). cancel synthesizes terminal node.report events for non-terminal nodes (per §7.7). reattach restarts a stale supervisor and replays unprocessed reports (per §7.6). Implements --dry-run (with dry_run_unsupported on create --parent-* per AGENTS-AI-FIRST-CLI §11) and --idempotency-key (on create). Verb is create, not new. **Depends on** state-schema-crate.
