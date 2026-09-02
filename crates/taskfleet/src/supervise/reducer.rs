//! `node.report` consumption for a parent supervisor (design.md §7.3).
//!
//! When a parent supervisor (a `fan-out` driver) observes a child's terminal
//! `node.report`, it advances the parent-side cursor of that child
//! (`last_processed_report_seq_by_child`) so the report is consumed exactly
//! once across supervisor restarts.
//!
//! The 0.2 cut removed the MID-RUN discussion/spinoff-proposal machinery: the
//! supervisor no longer derives `discussion.opened` / `spinoff.proposed` events
//! from the report's `discussion_items[]` / `spinoff_proposals[]`. Those fields
//! still ride the terminal report (kept), but they are surfaced at the round/PO
//! level rather than projected into per-run discussion/spinoff state.

use serde_json::{json, Value};

use taskfleet_core::{append_and_apply_unlocked, NodeId, RunId, RunLock, RunPaths, Status};

use crate::error::CliError;
use crate::supervise::state::SupervisorState;

/// Process one `node.report` from `child_run_id` against `parent_paths`,
/// holding the parent run's `flock` for the cursor-advance write.
///
/// `parent_node_id` is the parent node whose agent spawned this child (the one
/// that accumulates child references in `last_processed_report_seq_by_child`).
/// `child_node_id` is the reporting node *inside the child run*, typically
/// `n-0001`.
///
/// On success it atomically advances the parent's
/// `last_processed_report_seq_by_child[child_run_id]` (in `state` and, via a
/// `supervisor.cursor_advanced` event, on the parent node projection). The
/// caller must then [`crate::supervise::state::save`] the updated state.
/// Returns `Ok(None)` if `report_seq <= cursor` (already processed) — a
/// fast-path replay guard.
pub fn process_node_report(
    parent_paths: &RunPaths,
    parent_node_id: &str,
    child_run_id: &str,
    child_node_id: &str,
    report_seq: u64,
    _report: &Value,
    state: &mut SupervisorState,
) -> Result<Option<()>, CliError> {
    // Validate every id at the boundary, BEFORE acquiring the lock or appending
    // any event. `parent_nid` is reused for the cursor-advance append; the child
    // ids are validated for their own sake (they key the cursor state).
    let parent_nid = NodeId::parse_str(parent_node_id)
        .map_err(|e| CliError::user("invalid_id", e.to_string()))?;
    RunId::parse_str(child_run_id).map_err(|e| CliError::user("invalid_id", e.to_string()))?;
    NodeId::parse_str(child_node_id).map_err(|e| CliError::user("invalid_id", e.to_string()))?;

    if let Some(prev) = state
        .last_processed_report_seq_by_child
        .get(child_run_id)
        .copied()
    {
        if report_seq <= prev {
            return Ok(None);
        }
    }

    // Hold the parent run's flock for the cursor-advance write. Using
    // `RunLock::acquire` rather than `with_lock` lets us return `CliError`
    // directly instead of going through `core::Error`.
    let guard = RunLock::acquire(&parent_paths.lock())
        .map_err(|e| CliError::system("io_error", e.to_string()))?;
    // Mint the witness proving the exclusive lock is held, to thread into the
    // unlocked append entry point below.
    let lock = guard.witness();
    // Advance the parent-side projection of the *child's* report cursor by
    // appending a `supervisor.cursor_advanced` event the reducer folds onto the
    // parent node's `last_processed_report_seq_by_child` map — rather than
    // writing the node projection directly. The event is the backing record, so
    // a from-scratch projection rebuild reproduces the cursor; the reducer's
    // monotonic guard makes a replayed event idempotent. The SupervisorState
    // file remains the in-memory cursor of record; `parent_nid` was validated
    // at function entry.
    append_and_apply_unlocked(
        &lock,
        parent_paths,
        "supervisor.cursor_advanced",
        Some(&parent_nid),
        None,
        json!({ "child_run_id": child_run_id, "report_seq": report_seq }),
    )
    .map_err(|e| CliError::system("io_error", e.to_string()))?;
    drop(guard);

    state
        .last_processed_report_seq_by_child
        .insert(child_run_id.to_string(), report_seq);
    Ok(Some(()))
}

/// Apply the child node's terminal status to the **parent**'s view of that
/// child. The supervisor calls this after `process_node_report` so the parent's
/// `nodes/<spawning>.json` records a snapshot of the child's outcome. The actual
/// child-side `nodes/n-0001.json` is updated by the child supervisor (or the
/// child's own reducer).
#[allow(dead_code)]
pub fn child_terminal_status_from_report(report: &Value) -> Status {
    let cancelled = report
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if cancelled {
        return Status::Cancelled;
    }
    // `Some(false)` (explicit failure) and `None` (missing field) both mean
    // Failed; listed separately to document the missing-field case.
    #[allow(clippy::match_same_arms)]
    match report.get("success").and_then(Value::as_bool) {
        Some(true) => Status::Done,
        Some(false) => Status::Failed,
        None => Status::Failed,
    }
}
