---
created: 2026-06-27
updated: 2026-06-27
type: bug
assignee: jari
status: done
priority: high
epic: taskfleet-mvp
labels: [cross-repo]
closed: 2026-06-27
commits:
- hash: 39e3b56
  summary: 'fix(worktree/create.sh): slash-handling + agent-startup-timeout (30s default)'
---

# create.sh: slash handling + agent-startup-timeout default

## Description

Slash bug (workmux normalises / → - in tmux window names; awk match used raw branch name) and agent-PID timeout too short (5s default) caused real failures from /orchestrate and parallel spinoff spawns. Fix landed in homebase commit 39e3b56: BRANCH_NAME_FLAT match + --agent-startup-timeout flag with 30s default. User decision B from session 2026-06-27.
