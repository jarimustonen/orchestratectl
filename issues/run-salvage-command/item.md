---
created: 2026-07-26
updated: 2026-07-31
type: feature
reporter: claude-code
status: open
priority: normal
related: ['@agent-death-strands-recoverable-work']
---

# orchestratectl run salvage: recover a dead agent's stranded work

## Description

Follow-up to @agent-death-strands-recoverable-work (option 2). The acceptance floor there added a machine-readable `recoverable_work` signal on agent-died FAILED reports, surfaced by `run show`/`run wait`. This issue adds the ergonomic recovery command an operator currently runs by hand.

## Summary
Add `orchestratectl run salvage <run-id>` that takes the preserved branch of a failed run whose report carries `recoverable_work.recoverable == true` and fast-forwards / cherry-picks it into a fresh worktree for review + merge — the manual salvage the /stint conductor did in the original incident.

## Requirements
- Read the run's terminal node report; refuse (informative error) unless `recoverable_work.recoverable == true` (or a re-computed clean-merge verdict holds).
- Default: stage the branch in a fresh review worktree; do NOT auto-merge.
- `--no-review`: direct fast-forward/merge into source. Auto-merge MUST NEVER land unreviewed work unless `--no-review` is explicitly passed.
- Re-verify clean-merge against CURRENT source at salvage time (the stamped verdict is a snapshot from death time; source may have moved).
- Respect the hot-path / state-integrity invariants (supervise + lock layer).

## Also in this follow-up bucket (from the option-1 llm-review, history/review-agent-death-strands-recoverable-work.md)
- Multi-node surfacing: `run show`/`run wait` only read n-0001; extend recoverability to fan-out/orchestrate child nodes.
- Hard timeouts on supervise git subprocesses run under the run lock (pre-existing; the reconcile probe + new recoverability probe both shell out under the exclusive lock).
- Typed report-extension validation / provenance marker for `recoverable_work` instead of raw-Value passthrough.

## Decisions

### 2026-07-31T18:06:58Z · @claude

Orphan-reconcile gap (observed 2026-07-31 stint): a recoverable branch preserved by the teardown gate becomes a lifecycle-less ORPHAN once its work lands via a different run (e.g. a retry-with-harvest). No auto-reconcile — the superseded worktree/branch lingers until a human removes it. run salvage should cover this: detect a preserved recoverable branch whose commits are now reachable from / superseded by the source branch and offer or auto cleanup. Relates to @stint-recoverable-death-retry-harvest.
