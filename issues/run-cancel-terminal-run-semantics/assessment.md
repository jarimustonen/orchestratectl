# Review assessment — run-cancel-terminal-run-semantics

Source: `/llm-review` with gemini-3.1-pro, gpt-5.5, claude-opus-4-7, deepseek-v4-pro
(1 round, 4 reviewers) on `crates/taskfleet-core/src/cancel.rs`, `crates/taskfleet-cli/src/run/cancel.rs`,
and the supporting `error.rs` / `events.rs` / `schema.rs` context. Full raw reviews saved to
`history/review-cancel.md`.

## Applied (FIX)

| # | Finding | Reviewers | Resolution |
|---|---------|-----------|------------|
| 1 | `live_node_ids` swallowed `read_dir` iterator errors via `filter_map(Result::ok)` — a transient `DirEntry` I/O fault silently skips a live node, then the run is marked cancelled with that node stranded. | all 4 | Propagate the error (`entry.map_err(Error::io)?`). |
| 2 | Blank/whitespace `--note` → `reason: ""` → reducer `CancelledRequiresReason` aborts mid-loop; retries reuse the bad note → run permanently un-cancellable. | all 4 | Normalize note once up front (trim + filter empty → default). Same trim for the `run.status` note field. |
| 3 | `last_report.is_some()` on a non-terminal node was bucketed as `nodes_already_terminal` — a lie that also *stranded* the node (reducer gates on status, so cancelling it would have repaired the anomaly). | all 4 | Dropped the `last_report` skip entirely; only status-terminal nodes go to `nodes_already_terminal`. Behaviour is identical for all reachable states and strictly better (repair, not strand) for the impossible one. |
| 4 | TOCTOU: `run_not_found` pre-check comment was wrong, and a run deleted in the pre-check→lock window leaked `io_error` instead of `run_not_found`. | all 4 | Kept the pre-check (it stops `RunLock::acquire` from creating `<run-dir>/.lock` for a bogus id), fixed the comment, and mapped a NotFound from `cancel_run` back to `run_not_found`. |
| 5 | Module docs over-claimed "atomically" / honest "by construction". | gemini, gpt-5.5, opus | Reworded: single-lock + convergent, **not crash-atomic**; serializes *cooperating* writers only. |
| 6 | Lexical node-id sort (`n-10000` < `n-9999`). | gemini, gpt-5.5, deepseek | Sort by numeric suffix; added a test. |
| 7 | Text output never surfaced `nodes_already_terminal`. | gpt-5.5, opus | Both text lines now report the already-terminal count. |

New tests: blank-note normalization (3 blank forms), numeric-order convergence.

## Deferred — spun off

- **[[cancel-idempotent-batch-append]]** — idempotency keys for synthesized cancel events
  (crash-retry can append duplicate `node.report` / `run.status`) **and** a batch-append
  primitive to collapse N fsyncs under one held lock (gpt-5.5, opus). Both pre-date this
  change; the single lock magnifies the perf angle. Out of scope (bigger run-lifecycle work).
- **[[cancel-enumerate-from-event-log]]** — enumerate live nodes by replaying `events.jsonl`
  rather than scanning `nodes/*.json`, closing the projection-lag hole where a created-but-
  unprojected node escapes cancellation (gpt-5.5, opus). Deeper event-sourcing concern shared
  by other read paths; matches pre-existing CLI behaviour.

## Deferred — NEEDS A DECISION (not auto-applied)

**Exit class of `run_already_terminal`: exit 2 vs exit 1.** All four reviewers + the codebase's
own closest sibling (`run_not_found` = `CliError::user`, exit 1) argue a precondition refusal on a
Done/Failed run is a *user* error (exit 1), not a *system* error (exit 2). **The task brief
explicitly pre-authorized exit 2** ("Defaults to use without asking: … error code
`run_already_terminal` … with exit 2"), so I honored the explicit instruction and shipped exit 2,
rather than silently overriding it on review. `error.rs` does describe exit 2 as
"refused-but-actionable (system/IO)", which is a partial textual defense, but `AGENTS-AI-FIRST-CLI.md`
says plainly "2 = system error". **Recommendation: flip to exit 1 (`CliError::user`)** for
consistency with `run_not_found`. One-line change in `crates/taskfleet-cli/src/run/cancel.rs` plus the
exit-code assertion in `cancel_done_run_is_refused_run_already_terminal`. Flagged for the user to
confirm.

## Not actioned (considered, rejected/minor)

- `node.report` overloaded as a cancel control event (gpt-5.5): a real design smell, but the
  reducer already drives terminal node state from reports and this matches the existing supervisor
  cancel path; a `node.cancelled` first-class event is a separate design change.
- `cancel_run_unlocked` is `pub` footgun (gpt-5.5, opus): kept `pub` to honor the task's explicit
  "expose a `cancel_run_unlocked`" and for symmetry with `append_and_apply_unlocked` (also `pub`,
  used cross-crate). A debug-assert lock check is a reasonable future hardening.
- `expected: "running|pending|blocked"` excludes `cancelled` (gpt-5.5): the task specified this
  exact string; cancelled is accepted via convergence but isn't a "you must be in this state" hint.
