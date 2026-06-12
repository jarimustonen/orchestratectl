//! Integration tests for the `run` subcommand family.
//!
//! Every test points the binary at a fresh `TempDir` via
//! `ORCHESTRATECTL_HOME` so the user's real `~/.orchestratectl/` is
//! never touched.

use std::process::Command;

use serde_json::Value;
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

fn create(home: &TempDir, kind: &str, title: &str) -> String {
    let v = run_ok(bin(home).args(["--json", "run", "create", "--kind", kind, "--title", title]));
    v["data"]["run_id"]
        .as_str()
        .expect("run_id is string")
        .to_string()
}

#[test]
fn create_then_list_then_show_then_cancel_flow() {
    let home = TempDir::new().unwrap();
    let run_id = create(&home, "spinoff", "integration");

    // list returns the just-created run with status pending and
    // node_count 0 (no node.created yet — that's the supervisor's job).
    let v = run_ok(bin(&home).args(["--json", "run", "list"]));
    let runs = v["data"]["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], run_id);
    assert_eq!(runs[0]["status"], "pending");
    assert_eq!(runs[0]["kind"], "spinoff");

    // show returns the full manifest under `manifest` and counters.
    let v = run_ok(bin(&home).args(["--json", "run", "show", &run_id]));
    assert_eq!(v["data"]["manifest"]["run_id"], run_id);
    assert_eq!(v["data"]["counts"]["nodes"], 0);

    // cancel emits run.status:cancelled. With 0 nodes, no synthesized
    // node.report events — `cancelled_nodes` must be empty.
    let v = run_ok(bin(&home).args(["--json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], false);
    assert_eq!(v["data"]["cancelled_nodes"].as_array().unwrap().len(), 0);

    // Idempotent re-cancel.
    let v = run_ok(bin(&home).args(["--json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], true);

    let v = run_ok(bin(&home).args(["--json", "run", "show", &run_id]));
    assert_eq!(v["data"]["manifest"]["status"], "cancelled");
}

#[test]
fn create_dry_run_does_not_touch_filesystem() {
    let home = TempDir::new().unwrap();
    let v = run_ok(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["supervisor"], "not-yet-spawned");
    // No runs dir should exist (root not initialized).
    assert!(!home.path().join("runs").exists());
}

#[test]
fn create_child_dry_run_is_unsupported() {
    let home = TempDir::new().unwrap();
    let parent = create(&home, "orchestrated", "parent");
    let (code, v) = run_fail(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
        "--dry-run",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "dry-run-unsupported");
}

#[test]
fn create_child_writes_child_spawned_to_parent_events() {
    let home = TempDir::new().unwrap();
    let parent = create(&home, "orchestrated", "parent");
    let v = run_ok(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "child",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
    ]));
    let child = v["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(v["data"]["parent_run_id"], parent);

    // Parent run's events.jsonl must contain a child.spawned record
    // naming the child's run_id. This is the §7.2 protocol — child.spawned
    // belongs on the *parent's* log, not the child's.
    let parent_events =
        std::fs::read_to_string(home.path().join("runs").join(&parent).join("events.jsonl"))
            .expect("parent events readable");
    let mut saw_child_spawned = false;
    for line in parent_events.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["kind"] == "child.spawned" && v["data"]["child_run_id"] == child {
            saw_child_spawned = true;
        }
    }
    assert!(
        saw_child_spawned,
        "parent events missing child.spawned for {child}: {parent_events}"
    );

    // Child run's events must contain run.created (NOT child.spawned).
    let child_events =
        std::fs::read_to_string(home.path().join("runs").join(&child).join("events.jsonl"))
            .expect("child events readable");
    assert!(
        child_events.lines().any(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["kind"] == "run.created"
        }),
        "child missing run.created: {child_events}"
    );
}

#[test]
fn create_with_idempotency_key_returns_same_run_id() {
    let home = TempDir::new().unwrap();
    let v1 = run_ok(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--idempotency-key",
        "abc",
    ]));
    let r1 = v1["data"]["run_id"].as_str().unwrap().to_string();

    let v2 = run_ok(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--idempotency-key",
        "abc",
    ]));
    assert_eq!(v2["data"]["run_id"], r1);
    assert_eq!(v2["data"]["idempotent_replay"], true);

    // Only one run materialized on disk.
    let count = std::fs::read_dir(home.path().join("runs")).unwrap().count();
    assert_eq!(count, 1);
}

#[test]
fn create_rejects_empty_title() {
    let home = TempDir::new().unwrap();
    let (code, v) = run_fail(bin(&home).args([
        "--json", "run", "create", "--kind", "spinoff", "--title", "   ",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "invalid_value");
}

#[test]
fn create_rejects_unbalanced_parent_flags() {
    let home = TempDir::new().unwrap();
    // Only --parent-run-id without --parent-node-id is rejected by clap
    // (requires=...) with the structured envelope.
    let (code, v) = run_fail(bin(&home).args([
        "--json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--parent-run-id",
        "some-id",
    ]));
    assert_eq!(code, 1);
    assert!(
        v["error"]["code"].is_string(),
        "error.code must be present: {v}"
    );
}

#[test]
fn show_missing_run_returns_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, v) = run_fail(bin(&home).args(["--json", "run", "show", "nope"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run-not-found");
    assert_eq!(v["error"]["invalid_value"], "nope");
}

#[test]
fn cancel_missing_run_returns_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, v) = run_fail(bin(&home).args(["--json", "run", "cancel", "nope"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run-not-found");
}

#[test]
fn reattach_records_event_and_idempotent_repeat() {
    let home = TempDir::new().unwrap();
    let run_id = create(&home, "spinoff", "x");
    let v = run_ok(bin(&home).args(["--json", "run", "reattach", &run_id]));
    assert_eq!(v["data"]["action"], "reattach-requested");

    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let count = events
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["kind"] == "supervisor.reattach-requested"
        })
        .count();
    assert_eq!(count, 1);

    // Repeat: a second reattach is a fresh event (no dedup at this layer).
    run_ok(bin(&home).args(["--json", "run", "reattach", &run_id]));
    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let count = events
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["kind"] == "supervisor.reattach-requested"
        })
        .count();
    assert_eq!(count, 2);
}

#[test]
fn reattach_missing_run_returns_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, v) = run_fail(bin(&home).args(["--json", "run", "reattach", "nope"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run-not-found");
}

#[test]
fn list_filters_by_kind_and_status() {
    let home = TempDir::new().unwrap();
    let a = create(&home, "spinoff", "a");
    let _b = create(&home, "orchestrated", "b");

    let v = run_ok(bin(&home).args(["--json", "run", "list", "--kind", "spinoff"]));
    let runs = v["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], a);

    // Filter that matches nothing returns an empty list (not an error).
    let v = run_ok(bin(&home).args(["--json", "run", "list", "--status", "done"]));
    assert!(v["data"]["runs"].as_array().unwrap().is_empty());
}

#[test]
fn list_when_root_missing_returns_empty() {
    let home = TempDir::new().unwrap();
    // No runs created — runs/ dir does not exist yet.
    let v = run_ok(bin(&home).args(["--json", "run", "list"]));
    assert!(v["data"]["runs"].as_array().unwrap().is_empty());
}
