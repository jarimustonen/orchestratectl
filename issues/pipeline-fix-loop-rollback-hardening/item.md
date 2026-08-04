---
created: 2026-08-01
updated: 2026-08-04
type: task
status: in-progress
priority: normal
related: ['@pipeline-fix-loop-provenance']
---

# Pipeline fix-loop rollback: transactional/audit hardening (deferred review items)

## Description

Follow-up to `@pipeline-fix-loop-provenance`. The 4-model `/llm-review` of that work
(history/review-pipeline-fix-loop-provenance.md) surfaced several real-but-lower-priority
items deferred from the landing PR. The landed design is **safe** — nothing merges to
`source_branch` without the final `evaluate_feature_floor` re-check on the actual rebuilt
tip, and `rebuild_integration` is now transactional (restores the intact branch on any
replay conflict/error) — so these are hardening / audit-fidelity improvements, not
correctness holes.

## Deferred items

- **B — graceful `rollback_conflict` report status.** A cherry-pick conflict during
  `rebuild_integration` currently returns a hard `PipelineError::Git` (structured
  `CliError`, but no `PipelineReport`). For autonomy, thread a terminal
  `rollback_conflict` status out of `rebuild_integration` → `trigger_re_spec` /
  verify-FIX path → `LoopExit::Terminal` so the orchestrator gets a report naming the
  chunk that failed to replay.
- **E — replayed-chunk provenance fidelity.** After a rebuild the kept chunk's report
  sets `commit == merge_commit` (linear replay, no no-ff merge) and keeps the old
  `FloorVerdict`, whose gated tree may differ from the replayed tree. Add explicit
  fields (e.g. `original_gated_commit` / `replayed_commit` / `replayed: bool`) instead of
  overwriting `merge_commit`, and consider re-gating replayed chunks against their new
  base (the feature-floor re-check is the current safety net).
- **F — merge-commit inside a chunk range.** `attempt_chunk` accepts any
  `is_ancestor(base, head)` history, including a chunk branch that merged something. `git
  cherry-pick base..commit` cannot replay a merge commit without `-m` → the rollback
  conflicts. Either reject non-linear chunk histories at gate time, canonicalize each
  chunk to one squash commit before recording provenance, or replay a stored tree delta.
- **H — deterministic committer identity for `cherry_pick`.** Same ambient-identity
  exposure as the existing `merge_no_ff` (git_at only sets `commit.gpgsign=false`). Set
  `-c user.name/-c user.email` in `git_at` (affects merge + cherry-pick) so a
  identity-less CI/sandbox can't fail commit creation.
- **L — carry the prior diff into verify-FIX / re-spec re-codes.** Item 3's re-code-amnesia
  fix only covers the floor re-code path inside one `run_code_stage` visit. When a
  verify-FIX / re-spec rolls a merged chunk back, capture its provenance diff
  (`git::diff(base..commit)`) before the rollback and seed `prior_diff` for the re-run.
- **G — durable provenance refs + empty cherry-pick handling.** Pin kept chunks'
  `commit` OIDs under `refs/pipeline/prov/*` before resetting `feat`, instead of relying
  on object-DB reachability (a mid-run `git gc`/`worktree prune` could drop them).
  Handle an empty cherry-pick (a kept chunk whose change is already present) via
  `--empty=drop` or an explicit skip, rather than a hard error.
- **O — cumulative attempt count in breaker messages.** The terminal breaker message
  reports the per-visit `seq`, which resets each code-stage visit; report the cumulative
  `(plan_rev, chunk_id, tier)` attempt count so a cross-visit exhaustion reads correctly.

## Acceptance Criteria

- [x] B: rollback conflict yields a `PipelineReport` with a `rollback_conflict` status naming the failed chunk
- [x] E: replayed-chunk provenance no longer overwrites `merge_commit`; audit distinguishes authored vs replayed commits (`ChunkReport.replayed` + authored `commit` kept, replayed oid in `merge_commit`)
- [x] F: non-linear chunk histories are rejected at gate time (`git::range_has_merge` in `attempt_chunk`)
- [x] H: cherry-pick/merge use a deterministic committer identity (`user.name`/`user.email` `-c` overrides in `git_at`)
- [x] L: verify-FIX re-codes carry the reverted chunk's prior diff (`pending_prior_diff` seeded before rollback → `run_code_stage`). Re-spec re-codes deferred (see below).
- [~] G: empty cherry-pick handled (`--empty=drop`). Durable `refs/pipeline/prov/*` pinning **deferred** → follow-up `pipeline-provenance-durable-refs`.
- [x] O: breaker messages report cumulative `(plan_rev, chunk, tier)` re-code count alongside the per-visit seq

### Deferred (filed follow-ups)

- **G (durable provenance refs)** → `pipeline-provenance-durable-refs`. Rationale: within one supervised run no pipeline path invokes `git gc --prune`/`worktree prune`, and git's `gc --auto` default `gc.pruneExpire=2.weeks.ago` never prunes the seconds-old orphaned authored commits a rollback produces — so object-DB reachability holds for the run's lifetime. Pinning adds ref create/cleanup lifecycle (incl. the preserved-branch path) for an unobserved failure mode; worth doing, not blocking. Empty-cherry-pick half is DONE here.
- **L for the re-spec path** → same follow-up note. A re-spec produces a whole new plan (chunk identity/brief may change), so carrying a prior authored diff into a re-spec re-code is lower-value and potentially misleading; the verify-FIX path (stable chunk identity) is the meaningful case and is implemented.
