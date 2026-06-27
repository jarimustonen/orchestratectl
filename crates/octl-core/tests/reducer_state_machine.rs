//! State-machine invariants for the reducer (reducer-state-machine-hardening):
//! the success-xor-cancelled requirement on `node.report` and the
//! terminal-state guard on every status reducer.

use octl_core::events::read_all_events;
use octl_core::{
    append_event_with_seq, apply_event, ensure_root, read_manifest, read_node_opt, run_dir, Error,
    Node, RunLock, RunPaths, Status,
};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Harness {
    _tmp: TempDir,
    paths: RunPaths,
    next_seq: u64,
}

impl Harness {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        ensure_root(root).unwrap();
        let run_id = "01jxsnap000000000000000000".to_string();
        let dir = run_dir(root, &run_id);
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            paths: RunPaths::new(dir, run_id).unwrap(),
            next_seq: 0,
            _tmp: tmp,
        }
    }

    /// Append + apply, panicking if the reducer rejects the event.
    fn append(&mut self, kind: &str, node_id: Option<&str>, data: Value) {
        self.try_append(kind, node_id, data).unwrap();
    }

    /// Append + apply, surfacing the reducer's `Result` to the caller.
    fn try_append(
        &mut self,
        kind: &str,
        node_id: Option<&str>,
        data: Value,
    ) -> octl_core::Result<()> {
        self.next_seq += 1;
        let seq = self.next_seq;
        let paths = &self.paths;
        RunLock::with_lock(&paths.lock(), || {
            append_event_with_seq(paths, seq, kind, node_id, None, data)?;
            let events = read_all_events(&paths.events())?;
            let ev = events.last().unwrap();
            apply_event(paths, ev)
        })
    }

    fn node(&self, node_id: &str) -> Node {
        read_node_opt(&self.paths, node_id).unwrap().unwrap()
    }

    fn bootstrap_node(&mut self) {
        self.append(
            "run.created",
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "fixture" }),
        );
        self.append("run.status", None, json!({ "status": "running" }));
        self.append(
            "node.created",
            Some("n-0001"),
            json!({ "kind": "spinoff", "task": "do the thing" }),
        );
        self.append(
            "node.status",
            Some("n-0001"),
            json!({ "status": "running" }),
        );
    }
}

/// Cancelled → late success report → no-op: status stays Cancelled and
/// `last_report` keeps the cancel payload (the late report does not even
/// decorate the projection).
#[test]
fn cancelled_node_ignores_late_success_report() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let cancel_report = json!({
        "success": false,
        "cancelled": true,
        "reason": "cancelled by user",
        "summary": "Run cancelled before agent reported.",
    });
    h.append("node.report", Some("n-0001"), cancel_report.clone());
    assert_eq!(h.node("n-0001").status, Status::Cancelled);

    // Late agent success payload arrives after the cancel landed.
    h.append("node.report", Some("n-0001"), json!({ "success": true }));

    let n = h.node("n-0001");
    assert_eq!(n.status, Status::Cancelled, "cancel must stick");
    assert_eq!(
        n.last_report,
        Some(cancel_report),
        "late report must not overwrite last_report"
    );
}

/// Done → late failure report → no-op: status stays Done, `last_report`
/// unchanged.
#[test]
fn done_node_ignores_late_failure_report() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let success_report = json!({ "success": true, "summary": "all good" });
    h.append("node.report", Some("n-0001"), success_report.clone());
    assert_eq!(h.node("n-0001").status, Status::Done);

    h.append("node.report", Some("n-0001"), json!({ "success": false }));

    let n = h.node("n-0001");
    assert_eq!(n.status, Status::Done, "done must stick");
    assert_eq!(n.last_report, Some(success_report));
}

/// Bare `{}` report expresses neither success nor cancellation → CorruptEventLog.
#[test]
fn bare_report_payload_is_corrupt() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let err = h
        .try_append("node.report", Some("n-0001"), json!({}))
        .unwrap_err();
    assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");

    // The node is untouched: still running, no report decorated.
    let n = h.node("n-0001");
    assert_eq!(n.status, Status::Running);
    assert_eq!(n.last_report, None);
}

/// `success: true` + `cancelled: true` is a contradiction → CorruptEventLog.
#[test]
fn success_and_cancelled_both_true_is_corrupt() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let err = h
        .try_append(
            "node.report",
            Some("n-0001"),
            json!({ "success": true, "cancelled": true, "reason": "x" }),
        )
        .unwrap_err();
    assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");

    let n = h.node("n-0001");
    assert_eq!(n.status, Status::Running);
    assert_eq!(n.last_report, None);
}

/// `run cancel <run>` settles the run, then a late `node.report` success
/// arrives — the cancel must stick at both the run and node level.
#[test]
fn run_cancel_then_late_report_keeps_cancel() {
    let mut h = Harness::new();
    h.bootstrap_node();

    // `run cancel` flips the run to cancelled and synthesizes a cancel
    // report on each live node (design.md §7.7).
    h.append("run.status", None, json!({ "status": "cancelled" }));
    h.append(
        "node.report",
        Some("n-0001"),
        json!({ "success": false, "cancelled": true, "reason": "cancelled by user" }),
    );
    assert_eq!(read_manifest(&h.paths).unwrap().status, Status::Cancelled);
    assert_eq!(h.node("n-0001").status, Status::Cancelled);

    // Agent was mid-write when cancel landed; its success report arrives late.
    h.append("node.report", Some("n-0001"), json!({ "success": true }));
    // A late `run.status running` (e.g. a stale supervisor) must also no-op.
    h.append("run.status", None, json!({ "status": "running" }));

    assert_eq!(
        read_manifest(&h.paths).unwrap().status,
        Status::Cancelled,
        "run cancel must stick"
    );
    assert_eq!(
        h.node("n-0001").status,
        Status::Cancelled,
        "node cancel must stick"
    );
}

/// `node.status` terminal guard: a settled node ignores a late status event.
#[test]
fn node_status_terminal_guard() {
    let mut h = Harness::new();
    h.bootstrap_node();

    h.append("node.report", Some("n-0001"), json!({ "success": true }));
    assert_eq!(h.node("n-0001").status, Status::Done);

    h.append(
        "node.status",
        Some("n-0001"),
        json!({ "status": "running" }),
    );
    assert_eq!(
        h.node("n-0001").status,
        Status::Done,
        "terminal node must not transition back to running"
    );
}

/// `run.status` terminal guard, standalone: a `Done` run ignores a late
/// status event (the run-cancel test only exercises the `Cancelled` state).
#[test]
fn run_status_terminal_guard() {
    let mut h = Harness::new();
    h.append(
        "run.created",
        None,
        json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "fixture" }),
    );
    h.append("run.status", None, json!({ "status": "running" }));
    h.append("run.status", None, json!({ "status": "done" }));
    assert_eq!(read_manifest(&h.paths).unwrap().status, Status::Done);

    h.append("run.status", None, json!({ "status": "running" }));
    assert_eq!(
        read_manifest(&h.paths).unwrap().status,
        Status::Done,
        "terminal run must not transition back to running"
    );
}

/// A conflicting terminal transition (Done → Cancelled via `node.status`) is
/// dropped by the guard, not applied — the first terminal status wins.
#[test]
fn conflicting_terminal_transition_is_noop() {
    let mut h = Harness::new();
    h.bootstrap_node();

    h.append("node.report", Some("n-0001"), json!({ "success": true }));
    assert_eq!(h.node("n-0001").status, Status::Done);

    h.append(
        "node.status",
        Some("n-0001"),
        json!({ "status": "cancelled" }),
    );
    assert_eq!(
        h.node("n-0001").status,
        Status::Done,
        "a conflicting terminal status must not overwrite the settled one"
    );
}

/// Strict boolean typing: a non-boolean `success` on a live node is corrupt
/// (rather than silently coerced to "missing").
#[test]
fn non_boolean_success_is_corrupt() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let err = h
        .try_append("node.report", Some("n-0001"), json!({ "success": "true" }))
        .unwrap_err();
    assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");
    assert_eq!(h.node("n-0001").status, Status::Running);
}

/// Strict boolean typing: a non-boolean `cancelled` is corrupt even when a
/// valid `success` is present — it must not be coerced to `false`.
#[test]
fn non_boolean_cancelled_is_corrupt() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let err = h
        .try_append(
            "node.report",
            Some("n-0001"),
            json!({ "success": false, "cancelled": "true" }),
        )
        .unwrap_err();
    assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");
    assert_eq!(h.node("n-0001").status, Status::Running);
}

/// Guard-before-validate: a malformed (bare `{}`) report against an already
/// terminal node is a clean no-op, not a `CorruptEventLog`. A dead event must
/// not be able to brick replay of a settled node's log.
#[test]
fn corrupt_report_against_terminal_node_is_noop() {
    let mut h = Harness::new();
    h.bootstrap_node();

    let report = json!({ "success": true, "summary": "done" });
    h.append("node.report", Some("n-0001"), report.clone());
    assert_eq!(h.node("n-0001").status, Status::Done);

    // Bare `{}` would be CorruptEventLog against a live node, but the node is
    // terminal, so the guard short-circuits before validation.
    h.try_append("node.report", Some("n-0001"), json!({}))
        .expect("malformed report against terminal node must be a no-op");

    let n = h.node("n-0001");
    assert_eq!(n.status, Status::Done);
    assert_eq!(n.last_report, Some(report), "last_report must be untouched");
}
