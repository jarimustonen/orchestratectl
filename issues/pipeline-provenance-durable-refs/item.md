---
created: 2026-08-04
updated: 2026-08-06
type: task
status: done
priority: normal
related: ['@pipeline-fix-loop-rollback-hardening']
closed: 2026-08-06
---

# Pipeline rollback: durable provenance refs + re-spec prior-diff carry

## Description

Follow-up to `@pipeline-fix-loop-rollback-hardening` (deferred sub-items).

### G — durable provenance refs

Pin kept chunks' authored `commit` OIDs under `refs/pipeline/prov/<run>/<chunk>` before
`rebuild_integration` resets `feat/<slug>` to the fork, instead of relying on object-DB
reachability. Add matching ref cleanup on teardown (including the preserved-branch path,
so a preserved unmerged branch's refs are kept while a merged run's are pruned).

Why deferred: within one supervised run no pipeline path invokes `git gc --prune` /
`git worktree prune`, and git's `gc --auto` default (`gc.pruneExpire=2.weeks.ago`) never
prunes the seconds-old orphaned authored commits a rollback produces — so object-DB
reachability holds for the run's lifetime and the exposure is essentially nil under
default git config. Real only if an external aggressive `git gc --prune=now` races the
rollback. Belt-and-suspenders, not a correctness hole.

### Optimal empty-cherry-pick handling

The parent issue handles an empty/redundant replay CONSERVATIVELY: it surfaces as a
`Conflict` → clean `rollback_conflict` terminal report (branch restored intact), not a
crash. `--empty=drop` was tried and reverted (it produced `base==commit` provenance, a
misleading `merge_commit` pointing at a neighbour, a second-rollback `base..base` bug, and
a git-version dependency — see `history/review-pipeline-rollback-hardening.md`).

The optimal behaviour — DROP the redundant commit and CONTINUE the rebuild — needs an
explicit no-op provenance/report state (e.g. `merge_commit: None` + a `replayed`/`dropped`
marker, and preserving the AUTHORED range in provenance so a later rollback re-applies the
chunk if its subsumer is gone), plus a git-version capability check for `--empty`. Do it
here alongside the durable-refs work.

### L — re-spec path prior-diff carry

`pending_prior_diff` seeding of a reverted chunk's prior diff is implemented for the
verify-FIX path (stable chunk identity). Extend it to the `trigger_re_spec` path if
useful. Lower value: a re-spec produces a whole new plan (chunk brief/identity may
change), so a carried authored diff can be stale/misleading; needs the re-spec DAG-diff
to map old→new chunk identity before a diff is worth carrying.

Files: `crates/taskfleet-cli/src/pipeline/live/{mod.rs,git.rs}`.

