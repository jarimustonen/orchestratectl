---
created: 2026-08-06
updated: 2026-08-10
type: task
status: done
priority: normal
commits:
- hash: 66865f2
  summary: carry PipelineReport on hard-failure Err path; cmd_run exits non-zero AND surfaces branch_preserved siblings
- hash: 74fd1af
  summary: review fixes — best-effort failure-report emit; drop dead PipelineFailure->CliError conv
closed: 2026-08-10
---

# Carry a PipelineReport with hard-failure Err so inv-5 preservation is auditable (genuine inv-5 fix)

## Description

**This is the genuine invariant-5 audit fix that `entirely-faithful-beast` was reaching for** (that issue's premise — that teardown discards committed work — was incorrect: teardown never deletes chunk branches, so committed sibling work survives; only the audit record is lost on the Err path).

On a hard PipelineError in a concurrent wave, committed sibling work survives on disk but the run report — and thus the branch_preserved audit — is discarded because run_pipeline_tiered returns Err and Run drops. So invariant-5 preservation is UNAUDITABLE on the Err path today.

Fix: make run_pipeline_tiered's Result carry a PipelineReport on the Err path (e.g. PipelineFailure { error, report }); render it in cmd_run with a non-zero exit. This lets a hard failure both exit non-zero AND surface the preserved siblings (branch_preserved), closing the invariant-5 audit gap without downgrading the exit code.

Scope: touches the pipeline Result type, every ?-propagation, cmd_run, and all hard-error tests — cross-cutting on hot/correctness-sensitive code, needs its own design + review.

Source: /llm-review of entirely-faithful-beast (openai redesign, gemini #1, opus #3, deepseek #1).
