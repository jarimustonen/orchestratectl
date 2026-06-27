//! Single-lock run cancellation.
//!
//! `run cancel` does three things under **one** held [`RunLock`]: refuse a run
//! that is already in a non-cancelled terminal state, synthesize a terminal
//! `node.report` for every still-live node, and append `run.status: cancelled`
//! once. Holding one lock for the whole operation serializes it against other
//! *cooperating* writers (those that honor the lock) so the node reads and the
//! node-report appends can't interleave — which is what made the pre-refactor
//! CLI loop both racy and prone to over-reporting `cancelled_nodes` (it pushed
//! a node id even when the per-node append landed after another process had
//! already settled the node, so the reducer dropped it). Under one lock the
//! node we read is the node we cancel, so the reported count is honest.
//!
//! This is **not crash-atomic**: each `append_and_apply_unlocked` is its own
//! durable append, so a crash or I/O error partway through can leave some nodes
//! cancelled and `run.status` not yet appended. Recovery is convergent — a
//! re-`cancel` of an already-`Cancelled` run scans the still-live stragglers
//! and finishes the job — not transactional rollback.

use serde_json::json;

use crate::error::{Error, Result};
use crate::events::append_and_apply_unlocked;
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::projections::{read_manifest, read_node_opt};
use crate::schema::{NodeId, Status};

/// Outcome of a [`cancel_run`] transaction. Lets a thin CLI wrapper report
/// honestly what actually changed: which live nodes it converged, which were
/// already settled (skipped, not double-reported), and whether the run itself
/// was already cancelled (a convergence-only no-op rather than a fresh cancel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOutcome {
    /// True when the run's manifest was already `Cancelled` on entry, so no
    /// `run.status: cancelled` event was appended. The call still scans and
    /// converges any straggler nodes (an interrupted earlier cancel), so this
    /// is a SUCCESS, not an error: "no-op: run was already cancelled,
    /// converged N additional nodes".
    pub run_was_already_cancelled: bool,
    /// Nodes that were live on entry and for which this call synthesized and
    /// durably appended a terminal cancel `node.report`. Honest: a node only
    /// lands here when its append actually applied under the held lock.
    pub nodes_cancelled: Vec<NodeId>,
    /// Nodes whose *status* was already terminal on entry and so were skipped —
    /// never double-reported as freshly cancelled.
    pub nodes_already_terminal: Vec<NodeId>,
}

/// Cancel a run in a single locked transaction. Acquires the run's
/// [`RunLock`] once for the whole operation, then delegates to
/// [`cancel_run_unlocked`].
///
/// # Errors
///
/// - [`Error::RunAlreadyTerminal`] if the run is `Done`/`Failed` — refused
///   without mutating state.
/// - I/O / corrupt-log errors from reading the manifest, listing nodes, or
///   appending events.
pub fn cancel_run(paths: &RunPaths, note: Option<&str>) -> Result<CancelOutcome> {
    RunLock::with_lock(&paths.lock(), || cancel_run_unlocked(paths, note))
}

/// The locked body of [`cancel_run`]. The **caller must already hold** the
/// run's [`RunLock`]; this is the sanctioned lock-held composition path so the
/// manifest read, the per-node read-then-append loop, and the final
/// `run.status` append all share one critical section (it calls
/// [`append_and_apply_unlocked`], never [`crate::append_and_apply_event`],
/// which would deadlock by re-locking).
pub fn cancel_run_unlocked(paths: &RunPaths, note: Option<&str>) -> Result<CancelOutcome> {
    let manifest = read_manifest(paths)?;

    // Refuse a non-cancelled terminal run BEFORE touching any node: cancelling
    // a Done/Failed run would synthesize node reports and append a
    // `run.status: cancelled` the reducer's terminal-state guard then drops,
    // so the CLI would claim a transition that never happened. An already-
    // `Cancelled` run is not refused — it falls through to converge stragglers.
    if manifest.status.is_terminal() && manifest.status != Status::Cancelled {
        return Err(Error::RunAlreadyTerminal {
            status: manifest.status,
        });
    }
    let run_was_already_cancelled = manifest.status == Status::Cancelled;

    // Normalize the cancel reason ONCE up front. An empty or whitespace-only
    // `--note` would otherwise flow into the synthesized report as `reason: ""`,
    // which the reducer rejects (`CancelledRequiresReason`) — aborting the whole
    // transaction mid-loop and, since retries reuse the same bad note, leaving
    // the run permanently un-cancellable. A blank note falls back to the
    // default.
    let reason = note
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cancelled by user");

    let mut nodes_cancelled = Vec::new();
    let mut nodes_already_terminal = Vec::new();

    for nid in live_node_ids(paths)? {
        // Read each node fresh under the held lock. Because no cooperating
        // writer can settle it between this read and the append below, a node we
        // see as live here is still live at append time — so pushing it to
        // `nodes_cancelled` can never over-report a reducer no-op.
        let Some(n) = read_node_opt(paths, &nid)? else {
            continue;
        };
        // Only a genuinely terminal *status* means "already settled". We do NOT
        // also skip a non-terminal node that happens to carry `last_report`
        // (a should-never-happen anomaly): the reducer gates on status, so
        // synthesizing a cancel report there still transitions — and silently
        // skipping it would strand a live node in a cancelled run while lying
        // that it was "already terminal".
        if n.status.is_terminal() {
            nodes_already_terminal.push(nid);
            continue;
        }

        let data = json!({
            "success": false,
            "cancelled": true,
            "reason": reason,
            "summary": "Run cancelled before agent reported.",
            "discussion_items": [],
            "spinoff_proposals": [],
            "wrap_up_recommendations": []
        });
        append_and_apply_unlocked(paths, "node.report", Some(nid.as_str()), None, data)?;
        nodes_cancelled.push(nid);
    }

    if !run_was_already_cancelled {
        let mut status_data = serde_json::Map::new();
        status_data.insert("status".into(), "cancelled".into());
        // Record the operator note only when one was actually supplied (the
        // trimmed, non-blank value); a blank `--note` leaves the field unset
        // rather than writing an empty string.
        if let Some(n) = note.map(str::trim).filter(|s| !s.is_empty()) {
            status_data.insert("note".into(), n.into());
        }
        append_and_apply_unlocked(
            paths,
            "run.status",
            None,
            None,
            serde_json::Value::Object(status_data),
        )?;
    }

    Ok(CancelOutcome {
        run_was_already_cancelled,
        nodes_cancelled,
        nodes_already_terminal,
    })
}

/// Enumerate candidate node ids by scanning `nodes/`. Reads the directory
/// rather than trusting `manifest.node_count` so a dropped event (listing vs.
/// counter drift) can't hide a live node from cancellation. A stem that is not
/// a well-formed node id can't be one of our projection files, so it is
/// skipped rather than failing the whole cancel.
///
/// A `read_dir` *iterator* error (a `DirEntry` that fails mid-scan: transient
/// I/O, `EMFILE`, a permission fault) is propagated, never silently dropped —
/// skipping it would hide a live node, mark the run `cancelled`, and strand
/// that node, the exact dishonesty this refactor removes.
///
/// Returns ids sorted by their numeric suffix (not lexically), so output and
/// the cancel order stay intuitive past the digit-width boundary where
/// `n-10000` would otherwise sort before `n-9999` (see [`NodeId`]).
fn live_node_ids(paths: &RunPaths) -> Result<Vec<NodeId>> {
    let nodes_dir = paths.nodes_dir();
    let entries = match std::fs::read_dir(&nodes_dir) {
        Ok(e) => e,
        // No nodes/ yet (run never reached create.sh) — nothing to cancel.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&nodes_dir, e)),
    };
    let mut ids: Vec<NodeId> = Vec::new();
    for entry in entries {
        let p = entry.map_err(|e| Error::io(&nodes_dir, e))?.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(id) = NodeId::parse_str(stem) {
            ids.push(id);
        }
    }
    // A validated `NodeId` is `n-` + ASCII digits (≤10, so it fits in u64); the
    // unwrap_or keeps the sort total even for a hypothetical unparseable body.
    ids.sort_by_key(|id| {
        id.as_str()
            .strip_prefix("n-")
            .and_then(|d| d.parse::<u64>().ok())
            .unwrap_or(0)
    });
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{append_and_apply_event, read_all_events};
    use tempfile::TempDir;

    fn fresh_run(tmp: &TempDir) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        RunPaths::new(dir, run_id).unwrap()
    }

    /// Drive a run to `count` live nodes (n-0001..) under a created manifest.
    fn bootstrap(paths: &RunPaths, count: usize) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        for i in 1..=count {
            let nid = format!("n-{i:04}");
            append_and_apply_event(
                paths,
                "node.created",
                Some(&nid),
                None,
                json!({ "kind": "spinoff" }),
            )
            .unwrap();
        }
    }

    fn node_status(paths: &RunPaths, nid: &str) -> Status {
        let id = NodeId::parse_str(nid).unwrap();
        crate::read_node(paths, &id).unwrap().status
    }

    #[test]
    fn cancel_running_run_converges_live_nodes_and_settles_run() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);

        let out = cancel_run(&paths, Some("stop")).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-0002"]
        );
        assert!(out.nodes_already_terminal.is_empty());
        assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_done_run_is_refused_without_mutation() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        // Settle the single node, then the run, to Done.
        append_and_apply_event(
            &paths,
            "node.report",
            Some("n-0001"),
            None,
            json!({ "success": true }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "done" }),
        )
        .unwrap();
        let before = read_all_events(&paths.events()).unwrap().len();

        let err = cancel_run(&paths, None).unwrap_err();
        assert!(
            matches!(
                err,
                Error::RunAlreadyTerminal {
                    status: Status::Done
                }
            ),
            "got {err:?}"
        );
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "a refused cancel must not append any event"
        );
        assert_eq!(crate::read_manifest(&paths).unwrap().status, Status::Done);
    }

    #[test]
    fn recancel_cancelled_run_converges_straggler_node() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // Simulate an interrupted cancel: run is Cancelled, but n-0002 is still
        // live (its node.report never landed).
        append_and_apply_event(
            &paths,
            "node.report",
            Some("n-0001"),
            None,
            json!({ "success": false, "cancelled": true, "reason": "x" }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "cancelled" }),
        )
        .unwrap();
        assert_eq!(node_status(&paths, "n-0002"), Status::Pending);

        let out = cancel_run(&paths, None).unwrap();
        assert!(out.run_was_already_cancelled);
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"],
            "only the straggler converges"
        );
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
    }

    #[test]
    fn recancel_fully_converged_run_is_a_clean_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        cancel_run(&paths, None).unwrap(); // first cancel converges everything
        let before = read_all_events(&paths.events()).unwrap().len();

        let out = cancel_run(&paths, None).unwrap();
        assert!(out.run_was_already_cancelled);
        assert!(out.nodes_cancelled.is_empty(), "nothing left to converge");
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "a fully-converged re-cancel appends nothing"
        );
    }

    #[test]
    fn already_terminal_node_is_not_over_reported() {
        // The honesty guard: a node already settled (terminal) on entry is
        // reported under `nodes_already_terminal`, never `nodes_cancelled`,
        // even though it sits in nodes/ alongside a live node.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // n-0001 finishes on its own (Done) before the cancel.
        append_and_apply_event(
            &paths,
            "node.report",
            Some("n-0001"),
            None,
            json!({ "success": true }),
        )
        .unwrap();

        let out = cancel_run(&paths, None).unwrap();
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"]
        );
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Done,
            "Done node untouched"
        );
        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
    }

    #[test]
    fn cancel_run_with_no_nodes_dir_settles_run_only() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();

        let out = cancel_run(&paths, None).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert!(out.nodes_cancelled.is_empty());
        assert!(out.nodes_already_terminal.is_empty());
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn blank_note_falls_back_to_default_reason_and_does_not_brick_cancel() {
        // A `--note ""` (or whitespace-only) must NOT flow an empty `reason`
        // into the synthesized report — that would be rejected by the reducer
        // mid-loop and leave the run permanently un-cancellable. It normalizes
        // to the default reason and the cancel completes cleanly.
        for blank in ["", "   ", "\n\t"] {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap(&paths, 1);

            let out = cancel_run(&paths, Some(blank)).unwrap();
            assert_eq!(
                out.nodes_cancelled
                    .iter()
                    .map(NodeId::as_str)
                    .collect::<Vec<_>>(),
                vec!["n-0001"],
                "blank note {blank:?} still converges the live node"
            );
            assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
            let report = crate::read_node(&paths, &NodeId::parse_str("n-0001").unwrap())
                .unwrap()
                .last_report
                .expect("cancel report recorded");
            assert_eq!(report["reason"], "cancelled by user");
        }
    }

    #[test]
    fn nodes_are_converged_in_numeric_not_lexical_order() {
        // Past the digit-width boundary, lexical order would place n-10000
        // before n-9999. The numeric sort keeps the reported order intuitive.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        for nid in ["n-9999", "n-10000", "n-0001"] {
            append_and_apply_event(
                &paths,
                "node.created",
                Some(nid),
                None,
                json!({ "kind": "spinoff" }),
            )
            .unwrap();
        }

        let out = cancel_run(&paths, None).unwrap();
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-9999", "n-10000"],
        );
    }
}
