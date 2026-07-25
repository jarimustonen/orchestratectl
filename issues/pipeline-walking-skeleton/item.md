---
created: 2026-07-25
updated: 2026-07-25
type: task
status: in-progress
priority: high
---

# T5 walking skeleton: end-to-end pipeline command (spec[Opus]→code[claude-deepseek]→floor-gate→verify[Opus]→merge) for a single feature, additive new command

## Description

## Outcome (fixed)

Delivered the additive `orchestratectl pipeline run --intent <str|file>
--source-branch <branch> [--files …] [--slug …] [--repo …] [--workdir …] [--keep]`
command (`crates/octl-cli/src/pipeline/live/`). It forks `feat/<slug>` off the
pinned source OID, captures the T3 floor baseline, drives spec[Opus/`claude`] →
code[`claude-deepseek` `CodeHarness`] → per-chunk T3 floor gate → merge → verify
[Opus] → feature-floor re-check → merge-to-source, and emits a structured report
(chunk floor verdicts, verify result, feature floor, decision envelopes with the
deciding tier, final commit). The deterministic floor is the hard gate — a
harness commit is validated (no no-op/empty/rewrite/lying/uncommitted) before the
floor, and only the floor-gated OID is ever merged; spec/verify side effects are
restored away. Additive: does not touch `run create` / the supervisor.

Orchestration is unit-tested with a stub harness + scripted spec/verify against a
real throwaway git repo (no network); the live e2e is gated behind
`OCTL_PIPELINE_LIVE=1`. `/llm-review` (4 models) ran on the diff; real
floor-integrity / worktree-isolation / tier-split findings were fixed (see
`history/review-pipeline-walking-skeleton.md`).

Deferred follow-ups filed: `pipeline-fix-loop`, `pipeline-tiered-triage`,
`pipeline-circuit-breakers`, `pipeline-parallel-chunks`,
`pipeline-run-create-wiring`, `pipeline-hardening`.

