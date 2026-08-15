---
created: 2026-08-15
updated: 2026-08-15
type: bug
status: fixed
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: 2a58bfe
  summary: preserve dirty worktrees + fail closed on non-merge teardown
- hash: 6ce72c8
  summary: non-force teardown removal + tri-state cleanliness (llm-review fixes)
closed: 2026-08-15
---

# Preserve a dirty worktree on non-merge (cancel/plain-success) teardown

## Description

From /llm-review of A6 (pre-existing, surfaced by the review). cleanup_node's SourceRelative teardown (cancel, plain success) checks only committed commits vs source (rev_list_count) before remove_worktree --force; it does not check worktree_is_clean, so uncommitted edits are discarded. The deleted git-reconcile path had a worktree-clean guard. Committed work is still protected (source-relative check + git branch -d), so this is uncommitted-only. Also: branch_has_unmerged_commits fails open on a git error (rev_list_count None -> proceeds; the -d backstop still refuses an unmerged branch, but the worktree is removed). Add a worktree_is_clean guard + fail-closed on git error for the non-merge teardown path.
