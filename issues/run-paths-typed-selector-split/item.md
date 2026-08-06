---
created: 2026-08-04
updated: 2026-08-06
type: improvement
status: done
priority: normal
closed: 2026-08-06
---

# Split run_paths into typed run_paths_exact + RunSelector for CLI-only prefix resolution

## Description


## Context

From `/llm-review` of `run-cancel-accept-unambiguous-prefix` (GPT-5.6-sol, Opus 4.7).

`run_paths` now folds prefix resolution into the single chokepoint used by ~28 call sites, including supervisor and event-data paths. All internal callers pass full 26-char ULIDs, so they short-circuit (`arg.len() >= RunId::LEN` → returned verbatim, no scan, no behavior change). But that is a **runtime invariant, not a type-level guarantee**: a future caller passing a `<26`-char valid-Crockford id (e.g. a truncated `child_run_id` from event data) would silently fuzzy-resolve to some run instead of erroring loudly — a confused-deputy risk.

Proposed: `run_paths_exact(root: &RunId) -> Result<RunPaths, CliError>` for internal/typed paths; parse CLI args into `enum RunSelector { Exact(RunId), Prefix(String) }` and resolve only at verb entry, then pass a typed `RunId` downstream. Keeps supervisors/reducers exact-only.

Deferred from the prefix PR as out-of-scope.
