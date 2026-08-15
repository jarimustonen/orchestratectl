---
created: 2026-08-15
updated: 2026-08-15
type: task
status: done
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: d45664c
  summary: per-node branch-preserving run cancel --node for fan-out
- hash: e7e6d5d
  summary: llm-review fixes — cancel_node self-roll-up on last node + terminal-run guard
closed: 2026-08-15
---

# Per-node run cancel for fan-out (branch-preserving)

## Description

Split off from `attention-required-run-surface` (A5). Design §2.5 lists
per-node `run cancel <node>` (branch-preserving) so a single stuck fan-out
child can be unblocked without killing the whole batch, with rollup
terminalizing the run once every node is `merged | failed | cancelled`.

## Why it was deferred

A5 delivered the visibility surface (`run wait` attention-required settle,
`run list` / `run show` resume context). Per-node cancel is a genuinely
separate concern, not a natural rider on that surface:

- It needs a NEW core transaction in the hot `crates/octl-core/src/cancel.rs`
  + reducer path — cancel ONE live node and append its terminal cancel
  `node.report`, then roll the run up to `Cancelled` ONLY when all remaining
  nodes are terminal (today's `cancel_run` cancels *every* live node and
  terminalizes the run in one shot). That conditional-rollup logic, its
  per-node idempotency keys, and the multi-node convergence tests are their
  own design.
- The single-worker case is already covered: for a spinoff / research /
  technical-decision run (`node_count == 1`, the common topology), whole-run
  `run cancel <run>` == cancelling its one node, and the typed outcome table
  already classifies `cancelled: true` → `TerminalOutcome::Cancelled` →
  `Teardown::SourceRelative` (branch + worktree preserved, invariant 5). So
  the branch-preserving cancel semantics exist and are correct; only the
  fan-out-specific *per-node* selectivity is missing.

## Done criteria

- `run cancel <run> --node <node-id>` (or `run cancel <run> <node-id>`)
  cancels exactly one live node, preserves its branch + worktree (source-
  relative teardown, invariant 5), and does NOT terminalize the run while
  other nodes are still live.
- Rollup terminalizes the run `cancelled`/`done`/`failed` once every node is
  terminal.
- Coordinates with the typed outcome table (`supervise::outcome`) — no new
  teardown branch that bypasses `TerminalOutcome::teardown`.
- Tests for the multi-node partial-cancel + eventual-rollup path.

Coordinates with [[typed-supervisor-outcomes]] and the A5 attention surface.

## Acceptance Criteria

- [x] `run cancel <run> --node <node-id>` cancels exactly one live node and
  preserves its branch + worktree via `TerminalOutcome::Cancelled` →
  `Teardown::SourceRelative` (no new teardown branch; source-relative path).
- [x] A per-node cancel does not terminalize the run while sibling nodes are
  live; when it settles the LAST live node it rolls the run up under the same
  lock (`cancelled`/`failed` per the three-way `aggregate_terminal_status`), so a
  dead supervisor never strands the run (llm-review C1).
- [x] Idempotent: a duplicate per-node cancel reports the node already-terminal
  and appends no second report; a crash-stranded prior cancel is converged.
- [x] The node set is resolved log-authoritatively (a crash-interrupted
  projection is still cancellable; an absent id is `node_not_found`).
- [x] Tests: core `cancel_node` unit tests, CLI integration (partial cancel +
  eventual rollup), supervise_gates end-to-end rollup-to-cancelled with
  non-force teardown; help snapshots + `octl-run-overview` SKILL + CHANGELOG.
- [x] `/llm-review` (3 models) + `/assess-findings` run; confirmed localized FIX
  findings applied (C1, C4, S2/S3, M2); `rollup_status` log-authoritative
  deferred to [[rollup-status-log-authoritative]].
- [x] Green gate: `cargo fmt`, `clippy` (no new warnings), `cargo test --workspace`,
  `cargo doc` (all green).
