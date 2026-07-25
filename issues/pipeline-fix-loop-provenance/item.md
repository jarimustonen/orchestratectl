---
created: 2026-07-25
updated: 2026-07-25
type: task
status: open
priority: normal
related: ['@pipeline-fix-loop']
---

# Pipeline fix loop: provenance-aware chunk rollback + cumulative re-code budget

## Description

Follow-up hardening surfaced by the 4-model /llm-review of pipeline-fix-loop (history/assessment-pipeline-fix-loop.md). The landed fix loop (RE_CODE_CHUNK + TRIGGER_RE_SPEC + circuit-breakers) is bounded and floor-gated, but has three known v1 limitations needing provenance tracking: (1) Provenance-aware rollback — a re-coded/re-spec-reverted chunk commits ON TOP of prior merged work and a removed chunk's code stays in feat; proper fix rebuilds the integration branch from the fork replaying only kept-done chunk commits (needs per-chunk merge-commit provenance; see the in-code comment in pipeline/live/mod.rs FIX branch and design section 7). (2) Cumulative per-chunk re-code budget — max_recode_per_chunk resets per code-stage visit so a chunk can be re-coded across verify iterations/re-specs beyond the nominal bound (whole loop still terminates via max_fix_iterations x max_respec); track a cumulative counter keyed by (plan_rev, chunk_id). (3) Re-code amnesia — the failed attempt worktree is deleted before retry so the re-brief loses the failing diff; serialize the diff into the re-brief.
