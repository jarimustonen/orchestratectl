---
created: 2026-08-06
updated: 2026-08-14
type: task
status: obsolete
priority: normal
closed: 2026-08-14
closed_by: agent-cut-pipeline-floor-harness-heavy
---

# Carry stale wave build diff and findings into rebase-and-fix re-brief

## Description


Follow-up from the `immoderately-dirty-cushion` review (optional item 3 in the
original issue).

When a wave-build-exhausted chunk is re-queued into `process_chunk_sequential`
(Phase 3 of `run_wave_concurrent`), and when the merge phase re-drives a
conflicting/floor-regressed built chunk, the re-brief receives only the original
`pending_prior_diff` / `pending_findings` — NOT the stale wave attempt's committed
diff or the floor findings that drove its (now-spent) re-code budget. The promoted
/ rebased model therefore re-implements from scratch without the context of the
working-but-blocked implementation it just produced.

**Task:** compute the stale attempt's diff (`base..commit`) before
`reconcile_preserved_wave_build` deletes its worktree, merge it + `r.recode_findings`
into per-chunk drain inputs, and thread them into the sequential re-run. Mirror the
existing `prior_diff` plumbing (`pending_prior_diff`).

## Resolution

### 2026-08-14T04:42:34Z · @agent-cut-pipeline-floor-harness-heavy

Superseded by the 0.2 subtractive cut (cut-pipeline-floor-harness-heavy): the code-pipeline subsystem (pipeline/*, floor/*) and the harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek) it targeted were deleted. Nothing left to harden/wire/triage. See docs/decisions/0001-thin-supervisor-vs-harden.md D3.
