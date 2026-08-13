---
created: 2026-07-26
updated: 2026-08-13
type: feature
reporter: claude-code
status: open
priority: normal
related: ['@agent-death-strands-recoverable-work']
labels: [rescope-0.2]
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

### 2026-07-31T18:32:41Z · @claude

Second real data point (ossctl stint #6, 2026-07-31): during a live /orchestrate campaign (prose-skills, 7 orchestrated children), the last-in-line child f-changelog committed complete green work (1 commit: SKILL.template.md + skill.rs CATALOG row) but its run stayed 'pending' — event log showed only run.created→supervisor.attached, NO run-merge attempt logged. Its branch was forked from main while 5 siblings advanced the integration branch, so its CATALOG row collided as a skill.rs union conflict. The /stint conductor manually salvaged: git merge of the child branch into the integration branch, union-resolve skill.rs (keep all rows), re-run green gate (fmt/clippy/261 tests incl §17 lockstep), commit. Reinforces this issue: (1) 'run salvage' would automate exactly this; (2) it is an ORCHESTRATED child, so the 'multi-node / orchestrate-child recoverability surfacing' bullet applies — run show/run wait surfaced no recoverability signal. NB: agent-skips-run-merge-idle-pending (fixed 2026-07-28) is the idle-unmerged net that should terminalize this; unclear whether it fired before the manual salvage — worth a maintainer check that the net covers orchestrated children forked from a since-moved base.

### 2026-08-13T11:10:43Z · @adr-decision-2

RE-SCOPE: Becomes the fenced manual resume/finish skill (A3) — generalized from dead-branch salvage to live-worktree resume: fence the stuck agent, then drive run merge from the worktree's git state or launch one fresh agent. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).


