---
created: 2026-08-15
updated: 2026-08-15
type: task
status: open
priority: normal
epic: lifecycle-architecture-review
---

# Raw-git self-merge then death is a false-failed run under the thin model

## Description

From /llm-review of A6: an agent that self-merges with raw git (not run merge) then dies has no merge.started transaction, so A2 recovery cannot complete it and the crash backstop marks the run failed (branch+worktree preserved, work is in source, not data loss). Accepted thin-model tradeoff. Decide whether to add a non-fatal 'branch appears merged, run not Done' attention signal (never auto-success) or enforce run merge.
