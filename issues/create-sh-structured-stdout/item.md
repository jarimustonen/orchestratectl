---
created: 2026-06-12
updated: 2026-06-13
type: chore
assignee: jari
status: done
priority: high
epic: orchestratectl-mvp
labels: [cross-repo]
closed: 2026-06-13
---

# Patch create.sh to emit structured JSON stdout (cross-repo)

## Description

**Cross-project dependency** — lands in the homebase repo where ~/.claude/skills/worktree/scripts/create.sh lives. Patch create.sh to emit a single JSON object on stdout (per design.md §8.1): {schema_version, type, branch, worktree_path, tmux_window, agent_pid_hint, workmux_session}. Human-readable echoes move to stderr. Exit-code contract: 0 success, 1 user error, 2 system error with structured error envelope on stderr. Partial side-effect cleanup on failure paths verified by manual test (interrupt during git worktree add, during workmux add, during tmux send-keys). ~10 lines of bash; benefits the existing skill family too (callers get parseable output instead of regex-scraping). **Must land before all-kinds-spawn can ship.** **Validation gate**: V1.
