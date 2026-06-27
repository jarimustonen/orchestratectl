---
created: 2026-06-12
updated: 2026-06-27
type: feature
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Reducer state-machine hardening: terminal-state guard + report invariants

## Description


`apply_node_report` (in `crates/octl-core/src/reducer.rs:300`) treats missing `success` AND `cancelled` as "no status change" and silently leaves the node in its prior status with `last_report` populated. `node report` rejects this shape now, but the reducer is the canonical gate — a future write path or a replayed corrupt log would still produce a dangling-terminal-state node.

Likewise `apply_run_status` / `apply_node_status` / `apply_node_report` unconditionally write the new status, so a node cancelled by `run cancel` and then reporting success via a late-arriving agent payload flips back to `done` — breaking cancellation semantics.

Two related changes belong here together:
1. **Require `success` XOR `cancelled`** on `node.report` — return `CorruptEventLog` otherwise. (`node-cli-read/handoff.md` D4.)
2. **Terminal-state guard** — make every status reducer a no-op when the current status is `Done | Failed | Cancelled`. Document on `Status` itself with an `is_terminal()` helper. (`node-cli-read/handoff.md` D5, `run-cli-read/handoff.md` D5.)

Once this lands, the CLI-side terminal check that GPT-5.5 / Opus suggested in `node report` becomes unnecessary — the reducer is the single source of truth and avoids the "what about supervisor's `event create` path" duplication problem.

Sources: `issues/node-cli-read/handoff.md` D4, D5; `issues/run-cli-read/handoff.md` D5.
