//! Integration tests for the `discussion` subcommand family.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
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
    serde_json::from_slice(&out.stdout).expect("stdout is valid JSON")
}

fn run_fail(cmd: &mut Command) -> (i32, Value) {
    let out = cmd.output().expect("spawn");
    assert!(!out.status.success(), "expected failure");
    let code = out.status.code().expect("exit code");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr has at least one line");
    let v: Value = serde_json::from_str(last).expect("error envelope JSON");
    (code, v)
}

fn write_json(home: &TempDir, name: &str, v: Value) -> PathBuf {
    let p = home.path().join(name);
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    p
}

fn create_run(home: &TempDir) -> String {
    let v = run_ok(bin(home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "disc-test",
    ]));
    v["data"]["run_id"].as_str().unwrap().to_string()
}

/// Seed a `node.created` then a `discussion.opened` so the projection
/// file under `discussions/<id>.json` exists.
fn seed_discussion(home: &TempDir, run_id: &str, discussion_id: &str, topic: &str) {
    let nc = write_json(home, "nc.json", json!({"kind": "spinoff"}));
    run_ok(bin(home).args([
        "--json",
        "event",
        "create",
        run_id,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        nc.to_str().unwrap(),
    ]));
    let disc = write_json(
        home,
        "disc.json",
        json!({
            "discussion_id": discussion_id,
            "node_id": "n-0001",
            "topic": topic,
            "severity": "discuss"
        }),
    );
    run_ok(bin(home).args([
        "--json",
        "event",
        "create",
        run_id,
        "--kind",
        "discussion.opened",
        "--from-file",
        disc.to_str().unwrap(),
    ]));
}

// ---------- list ----------

#[test]
fn list_returns_open_discussion() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    let v = run_ok(bin(&home).args(["--json", "discussion", "list", &run_id]));
    let arr = v["data"]["discussions"].as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["discussion_id"], "d-01ONE");
    assert_eq!(arr[0]["status"], "open");
}

#[test]
fn list_filters_by_status() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    // Resolve it.
    run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "keep",
    ]));

    let open =
        run_ok(bin(&home).args(["--json", "discussion", "list", &run_id, "--status", "open"]));
    assert_eq!(open["data"]["discussions"].as_array().unwrap().len(), 0);

    let resolved = run_ok(bin(&home).args([
        "--json",
        "discussion",
        "list",
        &run_id,
        "--status",
        "resolved",
    ]));
    let arr = resolved["data"]["discussions"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["status"], "resolved");
}

#[test]
fn list_unknown_run_is_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, err) =
        run_fail(bin(&home).args(["--json", "discussion", "list", "01J0000000000000000000000X"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "run_not_found");
}

#[test]
fn list_empty_when_no_discussions_dir() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let v = run_ok(bin(&home).args(["--json", "discussion", "list", &run_id]));
    assert_eq!(v["data"]["discussions"].as_array().unwrap().len(), 0);
}

// ---------- show ----------

#[test]
fn show_returns_projection() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    let v = run_ok(bin(&home).args(["--json", "discussion", "show", &run_id, "d-01ONE"]));
    assert_eq!(v["data"]["discussion_id"], "d-01ONE");
    assert_eq!(v["data"]["topic"], "first topic");
    assert_eq!(v["data"]["status"], "open");
}

#[test]
fn show_missing_discussion_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) =
        run_fail(bin(&home).args(["--json", "discussion", "show", &run_id, "d-NOPE"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_not_found");
}

#[test]
fn show_invalid_id_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) =
        run_fail(bin(&home).args(["--json", "discussion", "show", &run_id, "../etc"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_id");
}

// ---------- resolve ----------

#[test]
fn resolve_writes_event_and_updates_projection() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    let v = run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "drop",
        "--note",
        "decided in standup",
    ]));
    assert!(v["data"]["seq"].as_u64().unwrap() > 0);
    assert_eq!(v["data"]["choice"], "drop");

    // Projection updated.
    let disc_path = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("discussions")
        .join("d-01ONE.json");
    let disc: Value = serde_json::from_slice(&std::fs::read(&disc_path).unwrap()).unwrap();
    assert_eq!(disc["status"], "resolved");
    assert_eq!(disc["resolution"], "drop");
    assert_eq!(disc["note"], "decided in standup");
    assert!(disc["resolved_at"].is_string());

    // Manifest open_discussions decremented.
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(home.path().join("runs").join(&run_id).join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["open_discussions"].as_u64().unwrap(), 0);
}

#[test]
fn resolve_same_choice_is_idempotent_noop() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "drop",
    ]));

    let events_before =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();

    let v = run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "drop",
    ]));
    assert_eq!(v["data"]["no_op"], true);
    assert!(v["data"]["seq"].is_null());

    let events_after =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    assert_eq!(events_before, events_after, "no-op must not append events");
}

#[test]
fn resolve_different_choice_is_conflict() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "drop",
    ]));

    let (code, err) = run_fail(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "keep",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_already_resolved");
    assert_eq!(err["error"]["expected"]["existing_resolution"], "drop");
}

#[test]
fn resolve_dry_run_does_not_touch_filesystem() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");

    let events_path = home.path().join("runs").join(&run_id).join("events.jsonl");
    let before = std::fs::read(&events_path).unwrap();

    let v = run_ok(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "drop",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);

    let after = std::fs::read(&events_path).unwrap();
    assert_eq!(before, after, "dry-run must not append to events.jsonl");

    // Projection still open.
    let disc_path = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("discussions")
        .join("d-01ONE.json");
    let disc: Value = serde_json::from_slice(&std::fs::read(&disc_path).unwrap()).unwrap();
    assert_eq!(disc["status"], "open");
}

#[test]
fn resolve_empty_choice_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-01ONE", "first topic");
    let (code, err) = run_fail(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-01ONE",
        "--choice",
        "   ",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn resolve_missing_discussion_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) = run_fail(bin(&home).args([
        "--json",
        "discussion",
        "resolve",
        &run_id,
        "d-NOPE",
        "--choice",
        "drop",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_not_found");
}
