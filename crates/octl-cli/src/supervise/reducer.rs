//! `node.report` consumption with deterministic-ID dedup (design.md §7.3).
//!
//! For each spinoff/discussion item carried by a child's `node.report`,
//! derive a stable ID from `(child_run_id, child_node_id, report_seq,
//! item_kind, item_index)`. Scan the parent's projection dir for an
//! existing file with that ID; if found, **skip emission** — the
//! supervisor crashed mid-batch on a prior run and is replaying. The
//! deterministic-ID formula plus the projection-existence check is the
//! exactly-once guarantee under crash recovery.
//!
//! Also marks the child's spawning-node `status: done | failed |
//! cancelled` based on the report payload.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use octl_core::{
    append_and_apply_unlocked, read_node_opt, write_node, RunLock, RunPaths, Status,
    STATE_SCHEMA_VERSION,
};

use crate::error::CliError;
use crate::supervise::state::SupervisorState;

/// `s-<10 hex chars of sha256>` / `d-<10 hex chars>`.
///
/// The `[..10]` slice in the issue spec is intentionally a chars-of-hex
/// slice: hex keeps the formula easy to verify by hand, and 10 hex chars
/// = 40 bits of entropy, plenty for per-run dedup (a few hundred items
/// at most). Design.md §1.4 shows base32 but the issue spec overrides
/// with the hex-style formula; we follow the issue.
pub fn deterministic_id(
    prefix: char,
    child_run_id: &str,
    child_node_id: &str,
    report_seq: u64,
    item_kind: &str,
    item_index: usize,
) -> String {
    let mut h = Sha256::new();
    h.update(child_run_id.as_bytes());
    h.update(b":");
    h.update(child_node_id.as_bytes());
    h.update(b":");
    h.update(report_seq.to_string().as_bytes());
    h.update(b":");
    h.update(item_kind.as_bytes());
    h.update(b":");
    h.update(item_index.to_string().as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(2 + 10);
    hex.push(prefix);
    hex.push('-');
    for &b in digest.iter().take(5) {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", b);
    }
    hex
}

// Fault-injection hook for V7's crash-recovery test. When set to
// `Some(n)`, `process_node_report` panics after writing the `n`-th
// derived event (1-indexed) but before recording the cursor. Always
// `None` outside tests. Thread-local makes the flip race-free under
// `cargo test`.
#[cfg(test)]
thread_local! {
    pub static FAULT_INJECT_AFTER_NTH: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Outcome of a single report-consumption call. Returned for tests and
/// for the supervisor's own bookkeeping (e.g. logging counts).
#[derive(Debug, Default, Clone)]
pub struct ReportConsumption {
    pub emitted_discussions: Vec<String>,
    pub emitted_spinoffs: Vec<String>,
    pub skipped_already_present: usize,
}

/// Process one `node.report` from `child_run_id` against `parent_paths`,
/// holding the parent run's `flock` for the entire write batch.
///
/// `parent_node_id` is the parent node whose agent spawned this child
/// (the one that should accumulate child references in
/// `last_processed_report_seq_by_child`). `child_node_id` is the
/// reporting node *inside the child run*, typically `n-0001`.
///
/// On success returns the per-batch counts and atomically advances the
/// parent's `last_processed_report_seq_by_child[child_run_id]` in
/// `state`. The caller must then [`crate::supervise::state::save`] the
/// updated state. Returns Ok(None) if `report_seq <= cursor` (already
/// processed) — a fast-path replay guard.
#[allow(clippy::too_many_arguments)]
pub fn process_node_report(
    parent_paths: &RunPaths,
    parent_node_id: &str,
    child_run_id: &str,
    child_node_id: &str,
    report_seq: u64,
    report: &Value,
    state: &mut SupervisorState,
) -> Result<Option<ReportConsumption>, CliError> {
    if let Some(prev) = state
        .last_processed_report_seq_by_child
        .get(child_run_id)
        .copied()
    {
        if report_seq <= prev {
            return Ok(None);
        }
    }

    let mut consumption = ReportConsumption::default();
    let mut emitted_count: usize = 0;

    // Hold the parent run's flock for the full write batch. Using
    // `RunLock::acquire` rather than `with_lock` lets the closure body
    // return `CliError` directly instead of going through `core::Error`.
    let _guard = RunLock::acquire(&parent_paths.lock())
        .map_err(|e| CliError::system("io_error", e.to_string()))?;
    {
        // Discussions first, then spinoffs — stable order makes the
        // deterministic-ID formula's `item_index` axis unambiguous.
        if let Some(items) = report.get("discussion_items").and_then(Value::as_array) {
            for (i, item) in items.iter().enumerate() {
                let id = deterministic_id(
                    'd',
                    child_run_id,
                    child_node_id,
                    report_seq,
                    "discussion",
                    i,
                );
                if parent_paths.discussion(&id).exists() {
                    consumption.skipped_already_present += 1;
                    continue;
                }
                let mut data = serde_json::Map::new();
                data.insert("discussion_id".into(), Value::String(id.clone()));
                if let Some(topic) = item.get("topic") {
                    data.insert("topic".into(), topic.clone());
                } else {
                    data.insert(
                        "topic".into(),
                        Value::String("(no topic supplied)".to_string()),
                    );
                }
                if let Some(sev) = item.get("severity") {
                    data.insert("severity".into(), sev.clone());
                }
                if let Some(opts) = item.get("options") {
                    data.insert("options".into(), opts.clone());
                }
                if let Some(ctx) = item.get("context") {
                    data.insert("context".into(), ctx.clone());
                }
                append_and_apply_unlocked(
                    parent_paths,
                    "discussion.opened",
                    Some(parent_node_id),
                    None,
                    Value::Object(data),
                )
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
                consumption.emitted_discussions.push(id);
                emitted_count += 1;
                fault_inject_check(emitted_count);
            }
        }
        if let Some(items) = report.get("spinoff_proposals").and_then(Value::as_array) {
            for (i, item) in items.iter().enumerate() {
                let id =
                    deterministic_id('s', child_run_id, child_node_id, report_seq, "spinoff", i);
                if parent_paths.spinoff(&id).exists() {
                    consumption.skipped_already_present += 1;
                    continue;
                }
                let mut data = serde_json::Map::new();
                data.insert("proposal_id".into(), Value::String(id.clone()));
                let title = item
                    .get("proposed_title")
                    .cloned()
                    .unwrap_or_else(|| Value::String("(no title)".into()));
                data.insert("proposed_title".into(), title);
                let kind = item
                    .get("proposed_kind")
                    .cloned()
                    .unwrap_or_else(|| Value::String("spinoff".into()));
                data.insert("proposed_kind".into(), kind);
                if let Some(r) = item.get("rationale") {
                    data.insert("rationale".into(), r.clone());
                }
                append_and_apply_unlocked(
                    parent_paths,
                    "spinoff.proposed",
                    Some(parent_node_id),
                    None,
                    Value::Object(data),
                )
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
                consumption.emitted_spinoffs.push(id);
                emitted_count += 1;
                fault_inject_check(emitted_count);
            }
        }

        // Mark the parent-side projection of the *child's* root node by
        // syncing the parent node's `last_processed_report_seq_by_child`
        // map onto its on-disk projection. The state file is the cursor
        // of record; the node-projection mirror is a debugging aid.
        if let Some(mut n) = read_node_opt(parent_paths, parent_node_id)
            .map_err(|e| CliError::system("io_error", e.to_string()))?
        {
            n.last_processed_report_seq_by_child
                .insert(child_run_id.to_string(), json!(report_seq));
            n.schema_version = STATE_SCHEMA_VERSION;
            write_node(parent_paths, &n)
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
        }
    }
    drop(_guard);

    state
        .last_processed_report_seq_by_child
        .insert(child_run_id.to_string(), report_seq);
    Ok(Some(consumption))
}

#[inline]
fn fault_inject_check(_emitted: usize) {
    #[cfg(test)]
    {
        FAULT_INJECT_AFTER_NTH.with(|c| {
            if let Some(n) = c.get() {
                if _emitted >= n {
                    // Clear so retry won't re-trigger.
                    c.set(None);
                    panic!("fault_inject: forced crash after {} emit(s)", _emitted);
                }
            }
        });
    }
}

/// Apply the child node's terminal status to the **parent**'s view of
/// that child. The supervisor calls this after `process_node_report`
/// so the parent's `nodes/<spawning>.json` records a snapshot of the
/// child's outcome. The actual child-side `nodes/n-0001.json` is
/// updated by the child supervisor (or the child's own reducer).
#[allow(dead_code)]
pub fn child_terminal_status_from_report(report: &Value) -> Status {
    let cancelled = report
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if cancelled {
        return Status::Cancelled;
    }
    match report.get("success").and_then(Value::as_bool) {
        Some(true) => Status::Done,
        Some(false) => Status::Failed,
        None => Status::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_id_is_stable() {
        let a = deterministic_id('d', "run-x", "n-0001", 7, "discussion", 2);
        let b = deterministic_id('d', "run-x", "n-0001", 7, "discussion", 2);
        assert_eq!(a, b);
        assert!(a.starts_with("d-"));
        assert_eq!(a.len(), 2 + 10);
    }

    #[test]
    fn deterministic_id_differs_per_axis() {
        let base = deterministic_id('s', "r", "n-0001", 1, "spinoff", 0);
        for diff in [
            deterministic_id('s', "r", "n-0001", 1, "spinoff", 1),
            deterministic_id('s', "r2", "n-0001", 1, "spinoff", 0),
            deterministic_id('s', "r", "n-0002", 1, "spinoff", 0),
            deterministic_id('s', "r", "n-0001", 2, "spinoff", 0),
        ] {
            assert_ne!(base, diff);
        }
    }
}
