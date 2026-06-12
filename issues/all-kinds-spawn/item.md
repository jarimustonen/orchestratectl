---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: open
priority: high
epic: orchestratectl-mvp
---

# All-kinds spawn (run create --kind <X>)

## Description

orchestratectl run create --kind <X> for all 8 kinds (code|spinoff|orchestrated|research|technical-decision|make-skill|fan-out|bugfix): validates kind, sets lifecycle from a kind→lifecycle table, calls ~/.claude/skills/worktree/scripts/create.sh --type <kind> with right args, **parses structured JSON stdout** (per design.md §8.1) to extract agent_pid_hint, tmux_window, worktree_path, branch, re-verifies agent_pid via tmux pane PID lookup, registers the node, returns {schema_version, run_id, dir, supervisor_pid}. Replaces the first-pass spinoff-spawn issue and generalizes it. **Depends on** run-cli-read, supervisor-process, **and create-sh-structured-stdout (cross-repo, hard prerequisite)**. **Validation gates**: V1, V2.
