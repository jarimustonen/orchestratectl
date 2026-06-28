//! Snapshot tests: drive a fresh run through fixture event streams and
//! snapshot the resulting projection files.

use insta::assert_json_snapshot;
use octl_core::{
    append_and_apply_event, ensure_root, read_discussion_opt, read_manifest, read_node_opt,
    read_spinoff_opt, run_dir, DiscussionId, NodeId, ProposalId, RunId, RunPaths,
};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Harness {
    _tmp: TempDir,
    paths: RunPaths,
}

impl Harness {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        ensure_root(root).unwrap();
        let run_id = "01jxsnap000000000000000000".to_string();
        let dir = run_dir(root, &RunId::parse_str(&run_id).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            paths: RunPaths::new(dir, run_id).unwrap(),
            _tmp: tmp,
        }
    }

    // Drive one event through the canonical mutation path (append + fold under
    // the run's flock). `seq` auto-recovers monotonically — identical to the
    // old hand-counted seq for these sequential fixtures, and these snapshots
    // cover projection files (never raw `seq`), so values are unchanged.
    fn append(&mut self, kind: &str, node_id: Option<&str>, data: Value) {
        // The fixtures carry node ids as `&str`; parse to the typed envelope id
        // the append API now takes.
        let node_id = node_id.map(|s| NodeId::parse_str(s).unwrap());
        append_and_apply_event(&self.paths, kind, node_id.as_ref(), None, data).unwrap();
    }
}

/// Normalize the `ts` and `*_at` fields out of any JSON value so snapshots
/// don't depend on wall-clock.
fn redact_times(mut v: Value) -> Value {
    redact_in_place(&mut v);
    v
}

fn redact_in_place(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if matches!(
                    k.as_str(),
                    "ts" | "created_at"
                        | "updated_at"
                        | "started_at"
                        | "opened_at"
                        | "resolved_at"
                        | "proposed_at"
                ) && val.is_string()
                {
                    *val = Value::String("<ts>".into());
                } else {
                    redact_in_place(val);
                }
            }
        }
        Value::Array(arr) => arr.iter_mut().for_each(redact_in_place),
        _ => {}
    }
}

fn snapshot_run(h: &Harness, name: &str, extras: &[(&str, &str)]) {
    let manifest = read_manifest(&h.paths).unwrap();
    let manifest_v = redact_times(serde_json::to_value(&manifest).unwrap());
    assert_json_snapshot!(format!("{name}__manifest"), manifest_v);
    for (label, node_id) in extras {
        let node_id = NodeId::parse_str(node_id).unwrap();
        if let Some(n) = read_node_opt(&h.paths, &node_id).unwrap() {
            let v = redact_times(serde_json::to_value(&n).unwrap());
            assert_json_snapshot!(format!("{name}__node_{label}"), v);
        }
    }
}

#[test]
fn run_node_report_spinoff_flow() {
    let mut h = Harness::new();
    let run_id = "01jxsnap000000000000000000";

    h.append(
        "run.created",
        None,
        json!({
            "kind": "spinoff",
            "lifecycle": "autonomous",
            "title": "investigate redirect loop",
        }),
    );
    h.append("run.status", None, json!({ "status": "running" }));
    h.append(
        "node.created",
        Some("n-0001"),
        json!({
            "kind": "spinoff",
            "task": "investigate /login redirect",
        }),
    );
    h.append(
        "node.status",
        Some("n-0001"),
        json!({ "status": "running" }),
    );
    h.append(
        "node.report",
        Some("n-0001"),
        json!({
            "success": true,
            "summary": "Root cause: stale cookie. Fixed.",
            "discussion_items": [],
            "spinoff_proposals": [{
                "proposed_title": "drop legacy cookie path",
                "proposed_kind": "spinoff",
                "rationale": "would tidy the auth surface"
            }],
            "wrap_up_recommendations": []
        }),
    );
    h.append(
        "spinoff.proposed",
        Some("n-0001"),
        json!({
            "proposal_id": "s-fxtrspnoff",
            "proposed_title": "drop legacy cookie path",
            "proposed_kind": "spinoff",
            "rationale": "would tidy the auth surface",
            "node_id": "n-0001",
        }),
    );
    h.append(
        "spinoff.approved",
        Some("n-0001"),
        json!({
            "proposal_id": "s-fxtrspnoff",
            "issue_slug": "drop-legacy-cookie-path",
        }),
    );

    snapshot_run(&h, "report_spinoff", &[("n0001", "n-0001")]);
    let s = read_spinoff_opt(&h.paths, &ProposalId::parse_str("s-fxtrspnoff").unwrap())
        .unwrap()
        .unwrap();
    let v = redact_times(serde_json::to_value(&s).unwrap());
    assert_json_snapshot!("report_spinoff__spinoff", v);

    // Sanity: run_id was carried through to all artifacts.
    assert_eq!(s.run_id.as_str(), run_id);
}

#[test]
fn discussion_open_and_resolve() {
    let mut h = Harness::new();

    h.append(
        "run.created",
        None,
        json!({
            "kind": "code",
            "lifecycle": "interactive",
            "title": "auth refactor",
        }),
    );
    h.append(
        "node.created",
        Some("n-0001"),
        json!({
            "kind": "code",
            "task": "rewrite auth middleware",
        }),
    );
    h.append(
        "discussion.opened",
        Some("n-0001"),
        json!({
            "discussion_id": "d-fxtrdscssn",
            "node_id": "n-0001",
            "topic": "should we drop the legacy cookie path?",
            "severity": "discuss",
            "options": ["keep", "drop", "feature-flag"],
            "context": "the legacy path predates the session-token rework",
        }),
    );
    h.append(
        "discussion.resolved",
        Some("n-0001"),
        json!({
            "discussion_id": "d-fxtrdscssn",
            "resolution": "drop",
        }),
    );

    let d = read_discussion_opt(&h.paths, &DiscussionId::parse_str("d-fxtrdscssn").unwrap())
        .unwrap()
        .unwrap();
    let v = redact_times(serde_json::to_value(&d).unwrap());
    assert_json_snapshot!("discussion__resolved", v);
    let m = read_manifest(&h.paths).unwrap();
    let mv = redact_times(serde_json::to_value(&m).unwrap());
    assert_json_snapshot!("discussion__manifest", mv);
}

#[test]
fn child_spawned_records_parent_child_link() {
    let mut h = Harness::new();

    h.append(
        "run.created",
        None,
        json!({
            "kind": "orchestrated",
            "lifecycle": "autonomous",
            "title": "epic foo",
        }),
    );
    h.append(
        "node.created",
        Some("n-0001"),
        json!({
            "kind": "orchestrated",
            "task": "drive epic foo",
        }),
    );
    h.append(
        "child.spawned",
        Some("n-0001"),
        json!({
            "child_run_id": "01jxchd0000000000000000000",
            "child_node_id": "n-0001",
            "child_kind": "spinoff",
            "child_title": "sub-task A",
        }),
    );
    h.append(
        "child.spawned",
        Some("n-0001"),
        json!({
            "child_run_id": "01jxche0000000000000000000",
            "child_node_id": "n-0001",
            "child_kind": "spinoff",
            "child_title": "sub-task B",
        }),
    );

    let n = read_node_opt(&h.paths, &NodeId::parse_str("n-0001").unwrap())
        .unwrap()
        .unwrap();
    let v = redact_times(serde_json::to_value(&n).unwrap());
    assert_json_snapshot!("child_spawned__parent_node", v);
}
