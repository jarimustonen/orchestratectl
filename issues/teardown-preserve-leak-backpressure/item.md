---
created: 2026-08-15
updated: 2026-08-15
type: bug
status: open
priority: normal
epic: lifecycle-architecture-review
labels: [deferred]
---

# Teardown fail-closed can leak a worktree forever on persistent git error

## Description

Follow-up from /llm-review of non-merge-teardown-dirty-worktree. The non-merge teardown guards (dirty-worktree, source-relative unmerged, non-force removal refusal) all fail closed: on a git error or a persistently-refused removal, cleanup_node records cleanup.branch_preserved and returns, retrying next tick. If the git error is PERMANENT (corrupt .git/worktrees/<name>, missing gitdir link, bad permissions, stale worktree lock, initialized submodule), every tick preserves and the worktree+branch persist indefinitely with no escalation. record_branch_preserved is idempotent by (run,node), so repeated failures add no new audit signal. Fail-closed is the correct data-loss posture (better leak than lose) and is the issue's explicit charter, so this is a separate observability/back-pressure concern: consider an attempt counter, a distinct cleanup.git_unavailable / cleanup.deferred event, or a once-per-run WARN so a chronic leak is visible in run show instead of only via git worktree list. Raised by openai/anthropic/deepseek.
