---
created: 2026-08-15
updated: 2026-08-15
type: task
status: in-progress
priority: high
epic: lifecycle-architecture-review
commits:
- hash: 831c9fa
  summary: surface attention-required runs without terminalizing (A5)
---

# Thin supervisor: surface attention-required runs

## Description

## Goal
Add the bounded visibility surface from `issues/lifecycle-architecture-review/design.md` §2.5 (A5): stuck non-terminal runs become visible as attention-required without terminalizing them.

## Context
Manual finish only works if a stuck run surfaces in the stint loop. `run wait --timeout` should return a distinct non-terminal attention-required result; `run list` / `run show` should expose pending age, last observed pid, worktree path, and a concise resume hint. Add branch-preserving per-node cancel if it is not already covered by the typed-outcome work.

Likely touches `crates/octl-cli/src/run/*`, run summary DTOs, and supervise status derivation.

## Done criteria
- `run wait --timeout` unblocks with an attention-required classification without mutating the run terminal.
- `run list` / `run show` expose enough fields for a PO-review to find and resume the stuck worktree.
- Per-node cancel is supported or explicitly delegated to the typed-outcome issue with no duplicate implementation.
- Full project green gate passes, including docs.
