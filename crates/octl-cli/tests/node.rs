//! Integration tests for `orchestratectl node {list,show,report}`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    c
}

fn run_ok(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout JSON")
}

fn run_fail(cmd: &mut Command) -> (i32, Value) {
    let out = cmd.output().expect("spawn");
    assert!(!out.status.success(), "expected failure");
    let code = out.status.code().expect("exit code");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    let last = stderr.lines().last().expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("envelope JSON");
    (code, v)
}

fn create_run(home: &TempDir) -> String {
    let v = run_ok(bin(home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "node-test",
    ]));
    v["data"]["run_id"].as_str().unwrap().to_string()
}

fn create_node(home: &TempDir, run_id: &str, node_id: &str) {
    let p = write_json(
        home,
        &format!("nc-{node_id}.json"),
        json!({"kind": "spinoff"}),
    );
    run_ok(bin(home).args([
        "--output",
        "json",
        "event",
        "create",
        run_id,
        "--kind",
        "node.created",
        "--node-id",
        node_id,
        "--from-file",
        p.to_str().unwrap(),
    ]));
}

fn write_json(home: &TempDir, name: &str, v: Value) -> PathBuf {
    let p = home.path().join(name);
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    p
}

#[test]
fn list_empty_run_returns_no_nodes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let v = run_ok(bin(&home).args(["--output", "json", "node", "list", &run_id]));
    assert_eq!(v["data"]["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["run_id"], run_id);
}

#[test]
fn list_returns_created_nodes_sorted() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0002");
    create_node(&home, &run_id, "n-0001");
    let v = run_ok(bin(&home).args(["--output", "json", "node", "list", &run_id]));
    let nodes = v["data"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["node_id"], "n-0001");
    assert_eq!(nodes[1]["node_id"], "n-0002");
    assert_eq!(nodes[0]["status"], "pending");
}

#[test]
fn list_status_filter() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    create_node(&home, &run_id, "n-0002");
    // Flip one node to running.
    let p = write_json(&home, "st.json", json!({"status": "running"}));
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.status",
        "--node-id",
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    let v = run_ok(bin(&home).args([
        "--output", "json", "node", "list", &run_id, "--status", "running",
    ]));
    let nodes = v["data"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["node_id"], "n-0001");
}

#[test]
fn list_unknown_run_rejected() {
    let home = TempDir::new().unwrap();
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "list",
        "01J0000000000000000000000X",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "run_not_found");
}

#[test]
fn list_invalid_run_id_rejected() {
    let home = TempDir::new().unwrap();
    let (code, err) = run_fail(bin(&home).args(["--output", "json", "node", "list", "../etc"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_id");
}

#[test]
fn show_returns_full_node_json() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let v = run_ok(bin(&home).args(["--output", "json", "node", "show", &run_id, "n-0001"]));
    assert_eq!(v["data"]["node_id"], "n-0001");
    assert_eq!(v["data"]["run_id"], run_id);
    assert_eq!(v["data"]["status"], "pending");
    assert_eq!(v["data"]["schema_version"], 1);
}

#[test]
fn show_unknown_node_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) =
        run_fail(bin(&home).args(["--output", "json", "node", "show", &run_id, "n-9999"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "node_not_found");
}

#[test]
fn report_appends_event_and_updates_node() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p = write_json(
        &home,
        "rep.json",
        json!({
            "success": true,
            "summary": "did the thing",
            "discussion_items": [],
            "spinoff_proposals": [],
            "wrap_up_recommendations": [],
        }),
    );
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert!(v["data"]["event_seq"].as_u64().unwrap() >= 2);

    // Projection reflects done + last_report.
    let node_path = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("nodes")
        .join("n-0001.json");
    let node: Value = serde_json::from_slice(&std::fs::read(&node_path).unwrap()).unwrap();
    assert_eq!(node["status"], "done");
    assert_eq!(node["last_report"]["summary"], "did the thing");
}

#[test]
fn report_dry_run_does_not_touch_filesystem() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let events_path = home.path().join("runs").join(&run_id).join("events.jsonl");
    let before = std::fs::read(&events_path).unwrap();

    let p = write_json(&home, "rep.json", json!({"success": true}));
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    assert!(v["data"]["event_seq"].is_null());
    let after = std::fs::read(&events_path).unwrap();
    assert_eq!(before, after, "events.jsonl must be untouched on dry-run");
}

#[test]
fn report_invalid_payload_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p = write_json(&home, "bad.json", json!({"summary": "no success"}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "schema_violation");
}

#[test]
fn report_unknown_node_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let p = write_json(&home, "rep.json", json!({"success": true}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-9999",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "node_not_found");
}

#[test]
fn report_idempotency_key_returns_existing_seq() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p = write_json(&home, "rep.json", json!({"success": true}));
    let v1 = run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));
    let seq1 = v1["data"]["event_seq"].as_u64().unwrap();

    let v2 = run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));
    assert_eq!(v2["data"]["event_seq"].as_u64().unwrap(), seq1);
    assert_eq!(v2["data"]["idempotent_replay"], true);

    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let count = events
        .lines()
        .filter(|l| l.contains("\"kind\":\"node.report\""))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn report_idempotency_conflict_on_payload_mismatch() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p1 = write_json(&home, "r1.json", json!({"success": true, "summary": "a"}));
    run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p1.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));

    let p2 = write_json(&home, "r2.json", json!({"success": true, "summary": "b"}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p2.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "idempotency_conflict");
}

#[test]
fn report_unknown_run_rejected() {
    let home = TempDir::new().unwrap();
    let p = write_json(&home, "rep.json", json!({"success": true}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        "01J0000000000000000000000X",
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "run_not_found");
}

#[test]
fn report_idempotency_conflict_on_node_mismatch() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    create_node(&home, &run_id, "n-0002");
    let p = write_json(&home, "rep.json", json!({"success": true}));
    run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0002",
        "--from-file",
        p.to_str().unwrap(),
        "--idempotency-key",
        "k1",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "idempotency_conflict");
}

#[test]
fn report_success_false_marks_node_failed() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p = write_json(&home, "rep.json", json!({"success": false}));
    run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    let node: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("nodes")
                .join("n-0001.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(node["status"], "failed");
}

#[test]
fn report_from_file_too_large_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let big = home.path().join("big.json");
    let mut s = String::from("{\"success\":true,\"summary\":\"");
    s.push_str(&"a".repeat(2 * 1024 * 1024));
    s.push_str("\"}");
    std::fs::write(&big, &s).unwrap();
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        big.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "from_file_too_large");
}

#[test]
fn event_create_cannot_bypass_node_report_validation() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    create_node(&home, &run_id, "n-0001");
    let p = write_json(&home, "bad.json", json!({"summary": "no success"}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.report",
        "--node-id",
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "kind_not_routable");
}

#[test]
fn list_emits_kebab_case_kind_and_status() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    // Use a multi-word kind variant so kebab-case matters.
    let p = write_json(&home, "tn.json", json!({"kind": "technical-decision"}));
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    let v = run_ok(bin(&home).args(["--output", "json", "node", "list", &run_id]));
    let nodes = v["data"]["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["kind"], "technical-decision");
    assert_eq!(nodes[0]["status"], "pending");
}

#[test]
fn report_invalid_run_id_rejected() {
    let home = TempDir::new().unwrap();
    let p = write_json(&home, "rep.json", json!({"success": true}));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        "../etc",
        "n-0001",
        "--from-file",
        p.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_id");
}
