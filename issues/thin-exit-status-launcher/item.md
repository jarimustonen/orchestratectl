---
created: 2026-08-15
updated: 2026-08-15
type: task
status: done
priority: high
epic: lifecycle-architecture-review
commits:
- hash: 6ce9379
  summary: thin launcher shim records real worker exit status (A1)
- hash: b1b2ab5
  summary: harden launcher shim per llm-review (signals, node-validate, spawn-fail fact, retry-clear)
closed: 2026-08-15
---

# Thin supervisor: record worker exit status via launcher shim

## Description

## Goal
Implement the thin launcher shim from `issues/lifecycle-architecture-review/design.md` §2.1 (A1): workers run through a small wrapper that records the real child exit status as a durable run event under the run lock.

## Context
This is part of the 0.2 thin-supervisor build. The supervisor must stop guessing completion from pid/pane/activity and instead consume told facts. The shim should distinguish:

- non-zero exit / killed by signal → typed failure path, branch/worktree preserved;
- exit 0 with an explicit merge event → success;
- exit 0 with no merge event → non-terminal attention-required / manual finish path, not auto-failed.

Likely touches `crates/taskfleet-cli/src/supervise/*` and the event/schema/reducer surface needed for the new event.

## Done criteria
- Worker launch goes through the shim for autonomous runs.
- The child exit code or signal is persisted durably and replay-safe.
- Existing spawn/merge tests are updated, plus at least one regression covering exit 0 without `run merge` not becoming done or failed.
- Full project green gate passes, including `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`.
