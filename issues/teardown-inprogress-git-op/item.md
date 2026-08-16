---
created: 2026-08-15
updated: 2026-08-16
type: improvement
status: wontfix
priority: normal
epic: lifecycle-architecture-review
labels: [deferred]
closed: 2026-08-16
closed_by: claude
---

# Teardown does not detect in-progress git operations (rebase/cherry-pick/sequencer)

## Description

Follow-up from /llm-review of detached-head-teardown-commit-loss (openai #4, opus C5). A clean worktree can still hold in-progress operation state (`rebase-merge`/`rebase-apply`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `MERGE_HEAD`, sequencer, bisect). Non-force `git worktree remove` refuses some (active rebase), and most conflict states leave a dirty tree the dirty guard already preserves; reviewers agreed this is mostly fail-closed today and NOT a confirmed committed-object-loss path. But a paused interactive rebase with a clean tree could silently destroy resumable state. Low urgency. Fix direction: a typed `git rev-parse --git-path {rebase-merge,rebase-apply,CHERRY_PICK_HEAD,sequencer,...}` existence probe on the non-merge path that preserves with a distinct audit reason while any operation is active.

## Resolution

### 2026-08-16T15:33:24Z · @claude

Suljettu: ei vahvistettu ongelma. Issue toteaa omassa tekstissään: 'pääosin fail-closed jo nyt ja EI vahvistettu committed-object-loss -reitti'. Vaatii keskeytetyn interaktiivisen rebasen puhtaalla työpuulla autonomisen agentin työpuussa — tilanne jota ei synny. Non-force removal torjuu aktiivisen rebasen jo nyt.
