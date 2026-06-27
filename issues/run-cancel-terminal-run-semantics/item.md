---
created: 2026-06-27
updated: 2026-06-28
type: improvement
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
---

# run cancel: handle Done/Failed runs and converge live nodes

## Description

Spun off from reducer-state-machine-hardening /llm-review (GPT-5.5 #2,#3,#4).

Now that the reducer enforces a terminal-state guard (a run in Done/Failed/Cancelled never transitions again), `run cancel` (crates/octl-cli/src/run/cancel.rs) has two gaps it didn't have before:

1. **Done/Failed runs**: `run cancel` only early-returns for `Status::Cancelled`. Cancelling a `Done`/`Failed` run now synthesizes node cancel reports, appends `run.status: cancelled` (which the reducer no-ops because the run is already terminal), and the CLI still prints `cancelled run ...` with success. The CLI claims a state change that the reducer refused. It should detect `manifest.status.is_terminal()` and either no-op with an accurate message or error `run_already_terminal`, without mutating nodes.

2. **Non-convergent re-cancel**: when the manifest is already `Cancelled`, the command returns immediately and never scans/cancels still-`Running` nodes. If a previous cancel was interrupted after `run.status: cancelled` but before all node reports, `run cancel` can't finish the job. Consider re-scanning live nodes on re-cancel.

Also (GPT-5.5 #4): the loop reads nodes and appends events without holding one RunLock across the whole transaction, and pushes node_id into `cancelled_nodes` even when the per-node append no-ops (node went terminal between read and append) — so `cancelled_nodes` can over-report. A core `cancel_run` API holding the lock for the full transaction would fix both the race and the honesty of the reported count.

Out of scope for the reducer-hardening issue (that one only touched reducer.rs/schema.rs + the is_terminal() refactor in cancel.rs). This is a CLI-semantics + possible core-API issue.

## Acceptance Criteria

- [x] `core::cancel_run(paths, note) -> CancelOutcome` (+ `cancel_run_unlocked`) holds ONE RunLock for the whole transaction
- [x] Cancelling a Done/Failed run is refused (`Error::RunAlreadyTerminal`) without mutating state; CLI surfaces `run_already_terminal` envelope
- [x] Re-cancelling an already-Cancelled run converges still-live straggler nodes (idempotent)
- [x] `cancelled_nodes` is honest — only nodes whose synthesize actually applied; already-terminal nodes reported separately
- [x] Build + tests + clippy + fmt clean (320 workspace tests green)
- [x] `/llm-review` (4 models) + assessment; FIX-class findings applied, deeper findings spun off

## Implementation Notes

- `crates/octl-core/src/cancel.rs` — new module: `cancel_run` / `cancel_run_unlocked`, `CancelOutcome`, `live_node_ids`.
- `crates/octl-core/src/error.rs` — `Error::RunAlreadyTerminal { status }`.
- `crates/octl-cli/src/run/cancel.rs` — thin wrapper; `run_not_found` pre-check + NotFound remap; `run_already_terminal` envelope.
- Review findings + decisions: see `assessment.md`. Raw reviews in `history/review-cancel.md` (gitignored).
- Spin-offs: `@cancel-idempotent-batch-append`, `@cancel-enumerate-from-event-log`.

## Open decision (deferred to user)

Exit class of `run_already_terminal`: shipped **exit 2** per the brief's explicit pre-authorized default,
but all 4 reviewers + the `run_not_found` sibling argue for **exit 1** (user error). One-line flip if confirmed.
See `assessment.md` → "NEEDS A DECISION".
