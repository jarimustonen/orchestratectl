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
//!
//! Two consistency properties beyond the single lock:
//!
//! - **Enumeration is from the event log, not the projection directory.** The
//!   node set is replayed from `events.jsonl` (the source of truth) rather than
//!   scanned from `nodes/*.json`. A `node.created` can be appended+fsynced while
//!   its projection write is crash-interrupted (`events.rs` documents the log
//!   leading the projections); a `nodes/` scan would silently drop that node,
//!   mark the run `cancelled`, and let a future `rebuild_projections` resurrect
//!   it as live under a `Cancelled` run. Walking the log closes that window.
//!   (The manifest's `node_count` is *also* a projection written in the same
//!   interrupted fold, so it is no more authoritative than `nodes/` — and it
//!   carries no node ids — which is why we replay the log rather than trust the
//!   counter.)
//!
//! - **Each synthesized event carries a deterministic idempotency key**
//!   (`run-cancel:<run_id>:node:<node_id>` and `run-cancel:<run_id>:run-status`).
//!   If a crash lands an append+fsync but interrupts the projection fold, the
//!   node/run still reads non-terminal, so a re-`cancel` would append a *second*
//!   logical-cancel event (duplicating it for auditors, metrics, and rebuild).
//!   The prior cancel events (scoped by `(kind, key)` for this run) are captured
//!   in the same replay pass, so instead of re-appending, the loop **re-folds
//!   the already-logged event** via [`apply_event`] — converging a projection
//!   the crash left non-terminal *without* a duplicate log line (a re-fold is a
//!   clean no-op when the projection already agrees). The whole transaction is
//!   then both non-duplicating and projection-convergent.
//!
//! What is still *not* fixed here (deliberately out of scope, tracked as
//! follow-ups): node/run **liveness is read from projections, not replayed from
//! the log**, so an unfolded non-cancel terminal event (e.g. a `node.report`
//! `success: true` fsynced but not applied before the crash) can still be
//! over-written by a freshly appended cancel and diverge on rebuild. Closing
//! that needs authoritative log-replayed liveness or a `rebuild_projections`
//! primitive (the parent issue gated this on exactly that). The full-payload
//! `read_all_events` materialization under the lock is likewise a known cost.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use crate::error::{Error, Result};
use crate::events::{append_and_apply_unlocked, read_all_events};
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::projections::{read_manifest, read_node_opt};
use crate::reducer::apply_event;
use crate::schema::{Event, NodeId, RunId, Status};

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
    /// Nodes this cancel transaction ensured are terminally cancelled: live
    /// nodes for which it synthesized and durably appended a terminal cancel
    /// `node.report` (and folded it), plus any node whose cancel `node.report` a
    /// prior interrupted cancel had already durably appended (matched by
    /// `(kind, idempotency_key)`) and which this call converged by *re-folding*
    /// that event rather than re-appending. Either way the node carries a
    /// terminal cancel in the source-of-truth log and its projection is folded
    /// (or, for a still-missing projection, will fold on rebuild — see the
    /// module docs); none is double-reported against a node that was already
    /// terminal on entry.
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
    let started = std::time::Instant::now();
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

    // One replay pass over the source-of-truth log: the authoritative node set
    // (immune to the projection crash window) and the prior cancel events
    // already recorded (so a prior interrupted cancel isn't duplicated — it is
    // re-folded instead).
    let CancelLedger {
        node_ids,
        prior_cancel,
    } = read_cancel_ledger(paths)?;

    let mut nodes_cancelled = Vec::new();
    let mut nodes_already_terminal = Vec::new();

    for nid in node_ids {
        // A node whose *status* is already terminal on entry is settled — report
        // it under `nodes_already_terminal`, never as freshly cancelled. Checked
        // before the idempotency key so a normally-converged re-cancel still
        // lands here (matching the pre-refactor projection-based behavior). We
        // do NOT also skip a non-terminal node that happens to carry
        // `last_report` (a should-never-happen anomaly): the reducer gates on
        // status, so synthesizing a cancel report there still transitions.
        if let Some(n) = read_node_opt(paths, &nid)? {
            if n.status.is_terminal() {
                nodes_already_terminal.push(nid);
                continue;
            }
        }
        // Reached for a live node OR one whose projection is missing entirely —
        // a `node.created` fsynced into the log but its projection write
        // interrupted (the crash window a `nodes/*.json` scan would silently
        // drop). Both must get a terminal cancel report so the log records the
        // node as cancelled and a future rebuild can't resurrect it as live.
        let key = node_cancel_key(&paths.run_id, &nid);
        if let Some(prior) = prior_cancel.get(&("node.report".to_owned(), key.clone())) {
            // A prior interrupted cancel already durably appended this node's
            // cancel report (fsynced) but may never have folded its projection,
            // so it can still read non-terminal here. Converge it WITHOUT a
            // duplicate log line by re-folding the already-logged event (a clean
            // no-op if the projection already agrees; for a still-missing
            // projection `apply_node_report` no-ops and the rebuild heals it).
            apply_event(paths, prior)?;
            nodes_cancelled.push(nid);
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
        append_and_apply_unlocked(paths, "node.report", Some(nid.as_str()), Some(&key), data)?;
        nodes_cancelled.push(nid);
    }

    if !run_was_already_cancelled {
        let key = run_status_cancel_key(&paths.run_id);
        if let Some(prior) = prior_cancel.get(&("run.status".to_owned(), key.clone())) {
            // A prior interrupted cancel already logged the terminal `run.status`
            // (fsynced before its manifest fold). Re-fold it to converge the
            // manifest instead of appending a duplicate `run.status: cancelled`.
            apply_event(paths, prior)?;
        } else {
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
                Some(&key),
                serde_json::Value::Object(status_data),
            )?;
        }
    }

    tracing::debug!(
        target: "octl_core::cancel",
        run_id = %paths.run_id,
        held_ms = started.elapsed().as_millis() as u64,
        nodes_cancelled = nodes_cancelled.len(),
        nodes_already_terminal = nodes_already_terminal.len(),
        "cancel transaction complete",
    );

    Ok(CancelOutcome {
        run_was_already_cancelled,
        nodes_cancelled,
        nodes_already_terminal,
    })
}

/// Cancel-relevant facts replayed from `events.jsonl` in one pass under the
/// held lock.
struct CancelLedger {
    /// Every node id a `node.created` event ever introduced, deduped and sorted
    /// by numeric suffix. The authoritative live-node *candidate* set (per-node
    /// status is then read from the projection): replayed from the source of
    /// truth, so it includes a node whose projection write was crash-interrupted
    /// — the exact node a `nodes/*.json` scan would miss.
    node_ids: Vec<NodeId>,
    /// Cancel events this run already logged, keyed by `(kind, idempotency_key)`
    /// and limited to this run's `run-cancel:<run_id>:` key namespace (first
    /// occurrence wins, mirroring [`crate::events::find_prior_with_key`]). The
    /// cancel loop looks an entry up by its deterministic key to (a) avoid
    /// re-appending a duplicate and (b) re-fold the event so a crash-stranded
    /// projection converges. Keying by `(kind, key)` — not the bare string —
    /// keeps a coincidental or forged key on an unrelated `kind` from masking a
    /// real cancel append.
    prior_cancel: HashMap<(String, String), Event>,
}

/// Replay `events.jsonl` once to build the [`CancelLedger`].
///
/// Reads through [`RunPaths::checked_events`] so a symlinked event log is
/// refused, matching the mutation path — the cancel decision must not be made
/// from content redirected outside the run tree. Uses [`read_all_events`], which
/// shares the crate's torn-tail policy: a crash-truncated final line is dropped
/// as an uncommitted partial write, while any *interior* unparseable line is
/// surfaced as [`Error::CorruptEventLog`] — so a corrupt log fails the cancel
/// loudly rather than silently dropping a node. A missing log yields an empty
/// ledger (run never appended an event — nothing to cancel).
///
/// Node ids are sorted by numeric suffix (not lexically), so output and the
/// cancel order stay intuitive past the digit-width boundary where `n-10000`
/// would otherwise sort before `n-9999` (see [`NodeId`]).
fn read_cancel_ledger(paths: &RunPaths) -> Result<CancelLedger> {
    let events = read_all_events(&paths.checked_events()?)?;
    let prefix = format!("run-cancel:{}:", paths.run_id.as_str());
    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut seen_nodes: HashSet<NodeId> = HashSet::new();
    let mut prior_cancel: HashMap<(String, String), Event> = HashMap::new();
    for ev in events {
        if ev.kind == "node.created" {
            if let Some(nid) = &ev.node_id {
                if seen_nodes.insert(nid.clone()) {
                    node_ids.push(nid.clone());
                }
            }
        }
        // Capture only this run's cancel events, keyed by (kind, key). The
        // `as_deref`/`map` releases the borrow before `ev` is moved below.
        let cancel_key = ev
            .idempotency_key
            .as_deref()
            .filter(|k| k.starts_with(&prefix))
            .map(str::to_owned);
        if let Some(key) = cancel_key {
            prior_cancel.entry((ev.kind.clone(), key)).or_insert(ev);
        }
    }
    // A validated `NodeId` is `n-` + ASCII digits (≤10, so it fits in u64); the
    // unwrap_or keeps the sort total even for a hypothetical unparseable body.
    node_ids.sort_by_key(|id| {
        id.as_str()
            .strip_prefix("n-")
            .and_then(|d| d.parse::<u64>().ok())
            .unwrap_or(0)
    });
    Ok(CancelLedger {
        node_ids,
        prior_cancel,
    })
}

/// Deterministic idempotency key for the synthesized cancel `node.report` of
/// one node. Stable in `(run_id, node_id)` so a re-`cancel` after a crash that
/// fsynced the report but never folded its projection finds the prior event and
/// does not append a duplicate logical-cancel.
fn node_cancel_key(run_id: &RunId, node_id: &NodeId) -> String {
    format!("run-cancel:{}:node:{}", run_id.as_str(), node_id.as_str())
}

/// Deterministic idempotency key for the run's terminal `run.status: cancelled`
/// event. Stable in `run_id` for the same crash-retry reason as
/// [`node_cancel_key`].
fn run_status_cancel_key(run_id: &RunId) -> String {
    format!("run-cancel:{}:run-status", run_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{append_and_apply_event, append_event_with_seq, read_all_events};
    use crate::lock::ACQUIRE_COUNT;
    use tempfile::TempDir;

    /// Count `node.report` events recorded in the log for one node id.
    fn report_count(paths: &RunPaths, nid: &str) -> usize {
        read_all_events(&paths.events())
            .unwrap()
            .iter()
            .filter(|e| {
                e.kind == "node.report" && e.node_id.as_ref().map(NodeId::as_str) == Some(nid)
            })
            .count()
    }

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

    #[test]
    fn cancel_synthesizes_report_for_node_with_missing_projection() {
        // The crash window this fix closes: a `node.created` was appended+fsynced
        // to the log, but its projection write (`nodes/n-NNNN.json`) was
        // interrupted. A `nodes/*.json` scan would not see n-0002 and would
        // cancel the run while leaving a created-but-never-cancelled node a
        // future rebuild could resurrect as live. Enumerating from the event log
        // sees it and synthesizes the cancel report.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // Delete n-0002's projection file, leaving its `node.created` event in
        // the log — exactly the interrupted-fold state.
        let n2 = NodeId::parse_str("n-0002").unwrap();
        std::fs::remove_file(paths.node(&n2)).unwrap();
        assert!(
            read_node_opt(&paths, &n2).unwrap().is_none(),
            "projection gone"
        );

        let out = cancel_run(&paths, Some("stop")).unwrap();
        // Both nodes are cancelled — the projection-present n-0001 AND the
        // projection-missing n-0002.
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-0002"],
            "the node with a missing projection is still cancelled"
        );
        assert!(out.nodes_already_terminal.is_empty());
        // The source-of-truth log now carries a terminal cancel report for the
        // node whose projection was missing — so a rebuild reconstructs it as
        // Cancelled, not live.
        assert_eq!(report_count(&paths, "n-0002"), 1);
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_takes_the_run_lock_exactly_once() {
        // The single-lock honesty guarantee: the whole transaction (N node
        // reports + the run.status append) runs under ONE flock acquisition, not
        // one per appended event. Spy on `RunLock::acquire` to prove it.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 5);

        // Bootstrap itself takes the lock once per append; only the cancel call
        // is under measurement.
        ACQUIRE_COUNT.with(|c| c.set(0));
        let out = cancel_run(&paths, Some("stop")).unwrap();
        assert_eq!(out.nodes_cancelled.len(), 5);
        assert_eq!(
            ACQUIRE_COUNT.with(std::cell::Cell::get),
            1,
            "cancel must take the run lock exactly once, not once per node (N+1)"
        );
    }

    #[test]
    fn cancel_does_not_duplicate_a_node_report_already_in_the_log() {
        // Crash-retry idempotency: a prior cancel appended+fsynced a node's
        // cancel `node.report` (carrying the deterministic key) but crashed
        // before folding its projection, so the node still reads live. A
        // re-cancel must NOT append a second logical-cancel event for it.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1); // run.created (seq 1) + node.created (seq 2)

        // Durably append the cancel report WITH the deterministic key, but
        // without folding it — the node stays Pending (live), modeling the
        // fsynced-but-not-applied window.
        let nid = NodeId::parse_str("n-0001").unwrap();
        let key = node_cancel_key(&paths.run_id, &nid);
        append_event_with_seq(
            &paths,
            3,
            "node.report",
            Some("n-0001"),
            Some(&key),
            json!({ "success": false, "cancelled": true, "reason": "x" }),
        )
        .unwrap();
        assert_eq!(node_status(&paths, "n-0001"), Status::Pending);
        assert_eq!(report_count(&paths, "n-0001"), 1);

        let out = cancel_run(&paths, None).unwrap();
        // The node converges (it is reported cancelled) but no duplicate report
        // is appended — the log still holds exactly one `node.report` for it.
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"],
        );
        assert_eq!(
            report_count(&paths, "n-0001"),
            1,
            "the already-logged cancel report must not be duplicated"
        );
        // Convergence: the crash-stranded projection is folded from the
        // already-logged event, so the node reads Cancelled (not the stale
        // Pending) even though no new event was appended for it.
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Cancelled,
            "the already-logged cancel must be re-folded, not just skipped"
        );
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_does_not_duplicate_run_status_already_in_the_log() {
        // The run-status analogue: a prior cancel fsynced `run.status: cancelled`
        // (with its deterministic key) but crashed before folding the manifest,
        // so the manifest still reads non-terminal. A re-cancel must not append a
        // second `run.status: cancelled`.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // run.created only (seq 1)

        let key = run_status_cancel_key(&paths.run_id);
        append_event_with_seq(
            &paths,
            2,
            "run.status",
            None,
            Some(&key),
            json!({ "status": "cancelled" }),
        )
        .unwrap();
        // Manifest never folded the cancel, so it is not terminal here.
        assert_ne!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
        let before = read_all_events(&paths.events()).unwrap().len();

        let out = cancel_run(&paths, None).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "no duplicate run.status appended when one is already logged"
        );
        // Convergence: the manifest is folded from the already-logged
        // `run.status: cancelled` instead of being left stale.
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled,
            "the already-logged run.status must be re-folded, not just skipped"
        );
    }
}
