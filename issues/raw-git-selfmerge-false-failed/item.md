---
created: 2026-08-15
updated: 2026-08-15
type: task
status: done
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: 7c847fc
  summary: 'feat(false-failed): surface raw-git self-merge death as non-terminal hint'
- hash: 4aab7fb
  summary: 'fix(false-failed): apply llm-review findings (base_sha gate, hint qualification, blind-spot docs)'
closed: 2026-08-15
closed_by: agent
---

# Raw-git self-merge then death is a false-failed run under the thin model

## Description

From /llm-review of A6: an agent that self-merges with raw git (not run merge) then dies has no merge.started transaction, so A2 recovery cannot complete it and the crash backstop marks the run failed (branch+worktree preserved, work is in source, not data loss). Accepted thin-model tradeoff. Decide whether to add a non-fatal 'branch appears merged, run not Done' attention signal (never auto-success) or enforce run merge.

## Resolution

### 2026-08-15T20:10:24Z · @agent

Implemented the 0.2 observability tradeoff: run show surfaces a read-time, non-mutating false_failed hint when a failed run's content is git-verified in source with no run merge recorded (raw-git self-merge then death), steering the user to run salvage. Never auto-succeeds (no branch-content heuristic, invariant 7 preserved). Regression tests cover no-auto-success + no destructive teardown + base_sha false-positive suppression. Multi-model llm-review + assess-findings applied; follow-ups filed: run-show-landed-git-timeout, shell-quote-dedup, enforce-run-merge.
