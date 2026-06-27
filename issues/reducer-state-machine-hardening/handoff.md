---
issue: reducer-state-machine-hardening
created: 2026-06-27
type: handoff
---

# Reducer state-machine hardening — handoff

## What landed

The reducer (`octl-core::reducer`) is now the canonical gate for two
invariants that were previously enforced only at CLI write time:

1. **success XOR cancelled on `node.report`** — `report_terminal_status`
   derives the terminal status and returns `CorruptEventLog` for a bare `{}`
   (neither outcome), the contradiction `success: true` + `cancelled: true`,
   or a non-boolean `success`/`cancelled` (strict typing via `optional_bool`).
2. **Terminal-state guard** — `apply_run_status`, `apply_node_status`, and
   `apply_node_report` are no-ops once the current status `is_terminal()`
   (`Done | Failed | Cancelled`). `Status::is_terminal()` lives on the type
   with the invariant documented. A late agent report cannot resurrect a node
   settled by `run cancel`.

`Node::last_report` is documented as frozen once terminal. The `run cancel`
settled-node skip was switched to `is_terminal()`.

## Two design calls worth knowing

- **Guard-before-validate** in `apply_node_report`: the terminal guard runs
  *before* payload validation, so a malformed dead report against a settled
  node is a clean no-op, not an error. This deviates from this issue's own
  spec ("validate parse-time, then guard") on the strength of 3/4 `/llm-review`
  consensus + replay-safety (a `CorruptEventLog` here would brick rebuild of a
  log `append_and_apply` already committed). All required `CorruptEventLog`
  cases target live nodes, so validation still runs for them. Locked in by
  `corrupt_report_against_terminal_node_is_noop`.
- **Conflict no-op, not error**: a conflicting transition out of a terminal
  state (e.g. `done` → `cancelled`) is dropped with a `warn` trace, not an
  `Err`, to keep replay idempotent. `trace_terminal_noop` logs `debug` for an
  idempotent same-status replay and `warn` for a real conflict.

## Rebase note

This branch was cut from `88b218b`, before main's `core-runpaths-store-run-id`
work (which added `RunPaths::new(root, run_id)` validation + the cross-run
guard in `apply_event`). It was rebased onto current main; the test harness
now uses the two-arg `RunPaths::new`. If a reviewer's diff appears to show the
cross-run guard "removed", that's a two-dot `git diff main` artifact, not a
real change — see `history/review-reducer-state-machine-hardening.md`.

## Spun off (not done here)

- **`run-cancel-terminal-run-semantics`** — `run cancel` only special-cases
  `Cancelled`; with the new guard it now lies for `Done`/`Failed` runs, is
  non-convergent on re-cancel, and can over-report `cancelled_nodes` under a
  read/append race. Needs CLI-semantics work and possibly a core `cancel_run`
  API that holds one lock for the whole transaction.
- **`append-and-apply-transactional-validation`** — `append_and_apply` makes
  the event durable before the reducer validates it, so a rejected event
  poisons the log for future replay. Pre-existing; affects all reducer
  validation, widened by this change. Fix: validate before the durable append,
  or stage-then-commit.

## Deferred review findings (with rationale)

- `last_report` suppression on a terminal node loses late agent telemetry —
  intentional and spec-mandated; documented on `Node::last_report` instead of
  changed.
- No replay-from-scratch / same-terminal-report convergence test — partially
  covered; deeper replay-equivalence testing belongs with the
  append-transactionality spin-off.
