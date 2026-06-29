---
created: 2026-06-28
updated: 2026-06-29
type: bug
status: fixed
priority: normal
closed: 2026-06-29
---

# Supervisor cleanup git worktree remove lacks --force; stray untracked file orphans worktree+branch

_Source: supervise cleanup_

## Description

During the /orchestrate smoke test (2026-06-28), Feature C`s supervisor failed to clean up its worktree:

```
supervisor cleanup: git worktree remove <path>: non-zero exit (continuing): fatal: ... contains modified or untracked files, use --force to delete it
supervisor cleanup: git branch -D wt/...: non-zero exit (continuing): error: cannot delete branch ... used by worktree at ...
```

Root cause: the agent left an untracked `.report.json` in the worktree, and the supervisor runs `git worktree remove` WITHOUT `--force`, so removal refused; the cascade then blocked `git branch -D`. Result: worktree AND branch orphaned, requiring manual `git worktree remove --force` + `git branch -D` by the orchestrator.

Features A and B cleaned up fine, so this only triggers when the worktree has untracked/modified files at teardown. Since the tracked work is already merged into the integration branch before teardown, untracked scratch is disposable.

Expected: supervisor cleanup should `git worktree remove --force` (or `git clean` then remove) so disposable untracked scratch does not orphan the worktree+branch. Found during /orchestrate end-to-end smoke test.
