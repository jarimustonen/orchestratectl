---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
priority: high
lane: supervise
---

# Pi spinoff batch creates stillborn runs after partial success

## Description

## Reproduction

On gertrud, `/stint-start` launched five `--kind spinoff --headless --harness pi` runs concurrently in homebase. Two runs created and completed. Three returned from `run create` only after the client timed out, but the CLI persisted normal success manifests with a supervisor pid initially. Each of the three then remained `pending` with `source_repo: null`, `source_branch: null`, `worktree_root: null`, and `node_count: 0`; `run wait` later correctly surfaced `stalled: true` and `error: "supervisor died before creating any worker node"`.

Affected run ids: `01m06xy235qx507eqtzry30ww8`, `01m06xy2364g0c672jfdtg95m0`, `01m06xy236hqsqt4tebbshepqw`. Successful peers: `01m06xy2356ftdw0jhddp6fkxg`, `01m06xy2359xhpkwkdehvjjqqm`.

This is related to fixed @run-wait-stillborn-run-not-detected, but distinct: that fix diagnoses a stillborn run promptly. The unresolved failure is that concurrent Pi spinoff creation can still produce them, so useful work never starts.

## Expected

Either each accepted `run create` establishes a worker node, or create fails/retries atomically with an actionable error. A concurrent headless Pi batch must not silently return successful-looking runs that have no source/worktree/node.

## Investigation

Determine whether this is an admission/concurrency/PTY issue, a Pi harness startup failure, or supervisor launch race. Preserve the prompt/worktree source contract and add a regression test if a deterministic seam exists.
