# Plan — reducer adopts a late explicit-merge report

## Root flaw
`reduce_node_report` drops ANY `node.report` against an already-terminal node as
a dead event (`last_report` untouched). A watchdog `agent-died` false positive
terminalizes a node; the later `via: "explicit-merge"` report is then swallowed,
so `any_node_merged_explicitly` never sees the merge and the supervisor can never
warrant teardown. The shipped fix compensated with an inline reclaim in
`run merge`, splitting teardown ownership.

## Fix (chosen: option 2 — explicit-merge overrides a prior terminal)
1. **Reducer adoption** (`octl-core/src/reducer.rs`). In `reduce_node_report`, when
   the node is already terminal AND the incoming report is a *confirmed successful
   explicit merge* (`via == "explicit-merge"`, `success == true`, not `cancelled`),
   ADOPT it: overwrite `last_report`, set status `Done`, refresh `updated_at`.
   Every other late report against a terminal node stays a dead event (unchanged).
   Idempotent: if `last_report` already equals this payload, no-op.
   - Replay-compat: old logs never carried an explicit-merge-vs-terminal sequence
     that we WANTED dropped — the only sequences affected are exactly the bug
     scenario, which now reduces to the correct (merged) projection. No new event
     kind, no schema/validator change, so every other log reduces byte-identically.
   - The adopted `last_report` (branch/source/via already in the payload) IS the
     structured merge receipt; no separate Node field added (keeps blast radius +
     schema/snapshots stable). Force `-D` stays gated on the confirmed merge via
     the existing `node_branch_merged` (`success == true` + explicit-merge/reconciled).

2. **`AppendResult.applied`** (`octl-core/src/events.rs`). Add `applied: bool` =
   "the reducer produced ≥1 projection op for THIS append" (false on idempotent
   replay). Private `append_and_apply_unlocked_reporting` returns `(seq, applied)`;
   public `append_and_apply_unlocked` stays `-> u64` (15+ callers untouched);
   `append_and_apply_event` fills the field. No user-facing serialization → no
   snapshot churn.

3. **`run merge` drops inline reclaim** (`octl-cli/src/run/merge.rs`). No more
   re-reading `last_report.via`. The reducer always adopts, so teardown is the
   supervisor's again. `ensure_report_consumer` gains a *terminal + teardown-
   warranted* reattach: `reattach ⟺ no live supervisor ∧ ever supervised ∧
   (¬terminal ∨ (terminal ∧ warranted ∧ fresh-adoption))`, where
   `warranted = autonomous ∨ any_node_merged_explicitly` and `fresh-adoption =
   result.applied`. So the swallowed path (supervisor already exited on a terminal
   run) reattaches a supervisor that runs the SAME `cleanup_terminal_nodes` and
   exits — single-owner teardown restored (invariant #5). Never-supervised skeleton
   runs (tests / `--skip-materialize`) return `NotSupervised` and are left alone —
   never a production run (every real worktree run emits `supervisor.started`).

4. **Remove the workaround** `reclaim_merged_worktree_branch`,
   `close_merged_node_window`, `branch_exists`, and their tests from
   `octl-cli/src/supervise/cleanup.rs`.

## Preservation gates (must stay intact)
- Blocked-report gate (`node_report_is_blocked`) and source-relative unmerged
  check unchanged — only a confirmed explicit merge adopts / force-deletes.
- Add/keep a test that a NON-explicit-merge terminal with unmerged commits still
  preserves branch + worktree.

## Proof
- New real-supervisor e2e test in `tests/e2e_spinoff.rs`: supervise a run,
  forge a watchdog `agent-died` terminal (supervisor rolls up + exits WITHOUT
  teardown), then `run merge` → reattached supervisor tears down worktree +
  branch. This is the staged watchdog-terminal-then-explicit-merge sequence.
- Rewrite the two forge-based `run_merge.rs` tests (`merge_reclaims…`,
  `merge_defers…`) for the new "reducer adopts, no inline teardown" behavior.

## Out of scope (noted)
- Reconciling `manifest.status` failed→done on the swallowed path (the
  `false-failed-after-merge` cosmetic symptom) — teardown fires regardless of
  failed vs done; run-status reconciliation on a terminal manifest is a separate,
  riskier change owned by the watchdog-reconcile path.
- Fixing the upstream watchdog false positive (`agent-died-merge-no-teardown-
  interactive` defect #1).

## Known residual (bounded, documented)
The reattach fixes the DEAD-supervisor swallowed path (the actual bug, e2e-proven).
There is a sub-tick LIVE-but-already-cleaned micro-window: an interactive run that
reached terminal WITHOUT a merge latches the supervisor's `cleaned` flag (teardown
not warranted, correct) and is about to exit via `all_work_done`; if `run merge`
lands in that exact sliver, `ensure_report_consumer` sees a live supervisor and
returns `Alive`, but the live supervisor won't re-run cleanup (the latch) and then
exits — so no teardown and no reattach. In practice the supervisor has long exited
by merge time (the real bug trace shows an ~18-minute gap), so this is essentially
unreachable; closing it would mean re-working the supervisor loop's `cleaned` latch
(a hot correctness path), deliberately left out of scope. The dead-supervisor path
— the documented failure — is fully closed by the reattach.
