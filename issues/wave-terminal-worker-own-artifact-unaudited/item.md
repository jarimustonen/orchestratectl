---
created: 2026-08-06
updated: 2026-08-10
type: task
status: in-progress
priority: normal
---

# Wave worker that commits then panics/errors leaves its own work unaudited (inv 5)

## Description

A wave build worker that commits real work and *then* panics or returns a hard PipelineError leaves its own branch (`<slug>/chunk-<id>`) on disk with committed work that no report names — an invariant-5 audit gap for the terminal worker itself. WaveJob::Error/Panicked discard the worker's artifact identity and its accumulated usages/recode_findings.

Fix: have the worker carry the last attempt's artifact identity (deterministic wt/branch, observed head) and audit state across the catch_unwind boundary (RAII/registry), then record a ChunkReport for the terminal worker even without vouching for its contents.

Source: /llm-review of entirely-faithful-beast (openai #5, opus #5, deepseek #2). Pre-existing since immoderately-dirty-cushion.
