---
created: 2026-08-05
updated: 2026-08-06
type: task
status: done
priority: normal
closed: 2026-08-06
---

# Adaptive tier promotion in concurrent wave builds

## Description

Follow-up from `pipeline-parallel-chunks` (concurrent DAG-wave scheduling).

The concurrent wave-build path (`build_chunk_in_wave` in
`crates/octl-cli/src/pipeline/live/mod.rs`) deliberately does NOT do adaptive tier
promotion (design §3) — a wave chunk that exhausts its floor re-code budget blocks
(preserved), whereas the strictly-sequential path (`process_chunk_sequential`)
promotes the chunk to the next model tier before giving up. Promotion mutates shared
run state (`chunk_tier`, `chunk_promotions`) and is awkward to thread through the
concurrent build.

**Consequence (flagged in the multi-model review):** a promotable chunk can succeed
with `--max-build-concurrency 1` but terminally block with `> 1`. The path is opt-in
(default 1) and the floor still gates correctly (no silent bad merge), so this is a
behavioral inconsistency, not a correctness hole — but it is a real UX surprise for
the opt-in path.

## Proposed fix

On wave-build exhaustion, instead of terminally blocking the chunk, re-queue it into
a sequential drain off the moved tip (`process_chunk_sequential`), which naturally
exercises promotion. The merge phase already uses this exact pattern for
rebase-and-fix. Care needed: the build-phase block already preserved the chunk's
worktree/report — the sequential re-run must reconcile / clean that preserved
attempt so it isn't left orphaned.

## Related review findings (same review, lower priority — fold in if cheap)

- Catch panics per wave-build worker (`catch_unwind`) and turn them into a blocked
  outcome so invariant-5 preservation still runs (a panic currently unwinds out of
  `run_wave_concurrent`, skipping the fold; teardown then removes the un-preserved
  worktrees).
- On a hard `PipelineError` from one build thread, the fold `?`-propagates and the
  other chunks' floor-green worktrees are torn down by `Run::Drop` rather than
  preserved (consistent with the sequential hard-error path, but worth revisiting).
- Optionally carry the stale build's diff into the rebase-and-fix re-brief
  (`prior_diff`) so the model keeps the context of the working implementation.

