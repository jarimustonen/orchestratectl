//! Single-transaction run cancellation.
//!
//! `run cancel` must do three things atomically: refuse a run that is already
//! in a non-cancelled terminal state, synthesize a terminal `node.report` for
//! every still-live node, and append `run.status: cancelled` exactly once. The
//! whole transaction runs under **one** held [`RunLock`] so the node reads and
//! the node-report appends can't interleave with a concurrent writer — which
//! is what made the pre-refactor CLI loop both racy and prone to over-reporting
//! `cancelled_nodes` (it pushed a node id even when the per-node append landed
//! after another process had already settled the node, so the reducer dropped
//! it). Under one lock the node we read is the node we cancel, so the reported
//! count is honest by construction.

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
    /// Nodes that were already in a terminal state (or already carried a
    /// terminal report) and so were skipped — never double-reported as
    /// freshly cancelled.
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

    let mut nodes_cancelled = Vec::new();
    let mut nodes_already_terminal = Vec::new();

    for nid in live_node_ids(paths)? {
        // Read each node fresh under the held lock. Because no other writer
        // can settle it between this read and the append below, a node we see
        // as live here is still live at append time — so pushing it to
        // `nodes_cancelled` can never over-report a reducer no-op.
        let Some(n) = read_node_opt(paths, &nid)? else {
            continue;
        };
        if n.status.is_terminal() {
            nodes_already_terminal.push(nid);
            continue;
        }
        // A non-terminal node should never already carry a terminal report
        // (reports are written only on the transition), but guard defensively:
        // synthesizing another would be a redundant event we'd then dishonestly
        // count as a fresh cancel.
        if n.last_report.is_some() {
            nodes_already_terminal.push(nid);
            continue;
        }

        let reason = note.unwrap_or("cancelled by user");
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
        if let Some(n) = note {
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
/// skipped rather than failing the whole cancel. Returns a sorted list for
/// deterministic output.
fn live_node_ids(paths: &RunPaths) -> Result<Vec<NodeId>> {
    let nodes_dir = paths.nodes_dir();
    let entries = match std::fs::read_dir(&nodes_dir) {
        Ok(e) => e,
        // No nodes/ yet (run never reached create.sh) — nothing to cancel.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&nodes_dir, e)),
    };
    let mut ids: Vec<NodeId> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                return None;
            }
            let stem = p.file_stem().and_then(|s| s.to_str())?;
            NodeId::parse_str(stem).ok()
        })
        .collect();
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
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
}
