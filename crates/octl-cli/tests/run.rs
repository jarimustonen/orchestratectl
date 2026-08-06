//! Integration tests for the `run` subcommand family.
//!
//! Every test points the binary at a fresh `TempDir` via
//! `ORCHESTRATECTL_HOME` so the user's real `~/.orchestratectl/` is
//! never touched.

use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

mod common;
use common::TestHome;

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
    let v = run_ok(bin(home).args([
        "--output", "json", "run", "create", "--kind", kind, "--title", title,
    ]));
    v["data"]["run_id"]
        .as_str()
        .expect("run_id is string")
        .to_string()
}

#[test]
fn create_then_list_then_show_then_cancel_flow() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "integration");

    // list returns the just-created run with status pending and
    // node_count 0 (no node.created yet — that's the supervisor's job).
    let v = run_ok(bin(&home).args(["--output", "json", "run", "list"]));
    let runs = v["data"]["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], run_id);
    assert_eq!(runs[0]["status"], "pending");
    assert_eq!(runs[0]["kind"], "spinoff");

    // show returns the full manifest under `manifest` and counters.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", &run_id]));
    assert_eq!(v["data"]["manifest"]["run_id"], run_id);
    assert_eq!(v["data"]["counts"]["nodes"], 0);

    // cancel emits run.status:cancelled. With 0 nodes, no synthesized
    // node.report events — `cancelled_nodes` must be empty.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], false);
    assert_eq!(v["data"]["cancelled_nodes"].as_array().unwrap().len(), 0);

    // Idempotent re-cancel.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], true);

    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", &run_id]));
    assert_eq!(v["data"]["manifest"]["status"], "cancelled");
}

#[test]
fn create_dry_run_does_not_touch_filesystem() {
    let home = TestHome::new();
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "x",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["supervisor"], "not-spawned-dry-run");
    // No runs dir should exist (root not initialized).
    assert!(!home.path().join("runs").exists());
}

#[test]
fn create_child_dry_run_is_unsupported() {
    let home = TestHome::new();
    let parent = create(&home, "orchestrated", "parent");
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
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
    assert_eq!(v["error"]["code"], "dry_run_unsupported");
}

#[test]
fn create_child_writes_child_spawned_to_parent_events() {
    let home = TestHome::new();
    let parent = create(&home, "orchestrated", "parent");
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
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
fn orchestrate_driver_exposes_discoverable_node_id() {
    let home = TestHome::new();

    // 1. Creating an orchestrate driver returns its driver node id in the
    //    envelope — no guessing required.
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "orchestrate",
        "--title",
        "campaign",
    ]));
    let driver = v["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(v["data"]["node_id"], "n-0001");
    assert_eq!(v["data"]["kind"], "orchestrate");
    assert_eq!(v["data"]["supervisor"], "orchestrator-in-main-conversation");

    // 2. The driver node is real on disk: run show counts it.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", &driver]));
    assert_eq!(v["data"]["manifest"]["node_count"], 1);
    assert_eq!(v["data"]["counts"]["nodes"], 1);

    // 3. node list surfaces exactly the driver node.
    let v = run_ok(bin(&home).args(["--output", "json", "node", "list", &driver]));
    let nodes = v["data"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["node_id"], "n-0001");
    assert_eq!(nodes[0]["kind"], "orchestrate");

    // 4. A child spawn pointed at the discovered node id succeeds — the
    //    whole reason the node has to exist.
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "orchestrated",
        "--title",
        "feature-a",
        "--parent-run-id",
        &driver,
        "--parent-node-id",
        "n-0001",
    ]));
    let child = v["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(v["data"]["parent_run_id"], driver);
    assert_eq!(v["data"]["parent_node_id"], "n-0001");

    // The parent's event log records the child.spawned under the driver node.
    let parent_events =
        std::fs::read_to_string(home.path().join("runs").join(&driver).join("events.jsonl"))
            .expect("driver events readable");
    assert!(
        parent_events.lines().any(|l| {
            let ev: Value = serde_json::from_str(l).unwrap();
            ev["kind"] == "child.spawned" && ev["data"]["child_run_id"] == child
        }),
        "driver events missing child.spawned for {child}: {parent_events}"
    );
}

#[test]
fn create_with_idempotency_key_returns_same_run_id() {
    let home = TestHome::new();
    let v1 = run_ok(bin(&home).args([
        "--output",
        "json",
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
        "--output",
        "json",
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

/// Regression for `idempotency-key-allowed-duplicate-run`: firing N `run
/// create` calls with the SAME `--idempotency-key` concurrently must yield
/// exactly ONE run, not one per call. Before the fix the key was persisted only
/// after a run fully materialized, so near-simultaneous calls all missed the
/// pre-create lookup and each spawned a distinct run. The atomic reservation
/// closes that window: exactly one caller materializes, the rest replay.
#[test]
fn concurrent_same_idempotency_key_creates_one_run() {
    let home = TestHome::new();
    const N: usize = 8;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(N));

    let handles: Vec<_> = (0..N)
        .map(|_| {
            let path = home.path().to_path_buf();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut cmd = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
                cmd.env("ORCHESTRATECTL_HOME", &path)
                    .env("OCTL_TEST_SKIP_MATERIALIZE", "1")
                    .args([
                        "--output",
                        "json",
                        "run",
                        "create",
                        "--kind",
                        "spinoff",
                        "--title",
                        "race",
                        "--idempotency-key",
                        "same-key",
                    ]);
                // Release all processes as close together as possible.
                barrier.wait();
                let out = cmd.output().expect("spawn");
                assert!(
                    out.status.success(),
                    "exit={:?} stderr={}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                );
                let v: Value = serde_json::from_slice(&out.stdout).expect("json");
                (
                    v["data"]["run_id"].as_str().unwrap().to_string(),
                    v["data"]["idempotent_replay"] == Value::Bool(true),
                )
            })
        })
        .collect();

    let results: Vec<(String, bool)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Exactly one run materialized on disk.
    let count = std::fs::read_dir(home.path().join("runs")).unwrap().count();
    assert_eq!(count, 1, "expected exactly one run dir, got {count}");

    // Every call resolved to the same run-id (the single materialized run).
    let materialized = std::fs::read_dir(home.path().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    assert!(
        results.iter().all(|(id, _)| *id == materialized),
        "all callers must return the one materialized run-id {materialized}; got {results:?}"
    );

    // Exactly one caller was the creator (no replay); the other N-1 replayed.
    let replays = results.iter().filter(|(_, r)| *r).count();
    assert_eq!(
        replays,
        N - 1,
        "exactly one creator + {} replays expected; got {replays} replays",
        N - 1
    );
}

/// Regression for the reservation-leak the review caught: an error AFTER the
/// idempotency key is reserved but BEFORE the run is durable must free the key
/// (via the drop-guard), so a later retry with the same key re-spawns cleanly
/// instead of replaying a phantom run that was never materialized. Here the
/// early error is a child spawn against a non-existent parent (`parent_not_found`
/// fires after `reserve`); the retry is a valid top-level create with the same
/// key, which must mint a NEW run, not `idempotent_replay` the leaked one.
#[test]
fn early_failure_after_reserve_frees_the_key() {
    let home = TestHome::new();
    let key = "leak-key";

    // A child create against a parent that does not exist: reserve succeeds,
    // then the parent-existence check fails. The guard must release the key.
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "orphan",
        "--parent-run-id",
        "01000000000000000000000000",
        "--parent-node-id",
        "n-0001",
        "--idempotency-key",
        key,
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "parent_not_found");
    // No run materialized, and the key file was released (not left behind).
    let runs = home.path().join("runs");
    let count = std::fs::read_dir(&runs).map_or(0, Iterator::count);
    assert_eq!(count, 0, "no run should have materialized");

    // Retry with the SAME key as a normal top-level create: because the key was
    // freed, this is a fresh creation — not an idempotent replay of a phantom.
    let v2 = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "retry",
        "--idempotency-key",
        key,
    ]));
    assert_ne!(
        v2["data"]["idempotent_replay"],
        Value::Bool(true),
        "a freed key must not replay: {v2}"
    );
    assert!(v2["data"]["run_id"].is_string());
    let count = std::fs::read_dir(&runs).unwrap().count();
    assert_eq!(count, 1, "exactly the retry's run exists");
}

#[test]
fn create_rejects_empty_title() {
    let home = TestHome::new();
    let (code, v) = run_fail(bin(&home).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "   ",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "invalid_value");
}

#[test]
fn create_rejects_unbalanced_parent_flags() {
    let home = TestHome::new();
    // Only --parent-run-id without --parent-node-id is rejected by clap
    // (requires=...) with the structured envelope.
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
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
    let home = TestHome::new();
    // A well-formed ULID that simply names no run → run_not_found.
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
        "run",
        "show",
        "01jzabsent0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run_not_found");
    assert_eq!(v["error"]["invalid_value"], "01jzabsent0000000000000000");
}

#[test]
fn show_malformed_run_id_returns_invalid_run_id() {
    let home = TestHome::new();
    // A malformed id is a distinct error class from a missing run.
    let (code, v) = run_fail(bin(&home).args(["--output", "json", "run", "show", "nope"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "invalid_run_id");
    assert_eq!(v["error"]["invalid_value"], "nope");
}

#[test]
fn cancel_missing_run_returns_run_not_found() {
    let home = TestHome::new();
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
        "run",
        "cancel",
        "01jzabsent0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run_not_found");
}

#[test]
fn cancel_malformed_run_id_returns_invalid_run_id() {
    let home = TestHome::new();
    // Uppercase ULID-shaped input is non-canonical → invalid_run_id.
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
        "run",
        "cancel",
        "01JZABSENT0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "invalid_run_id");
}

#[test]
fn cancel_resolves_unambiguous_run_id_prefix() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "prefix");
    // A prefix (first 12 chars) that names exactly one run resolves to the full
    // id and cancels it — the payload echoes the resolved full id, not the prefix.
    let prefix = &run_id[..12];
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", prefix]));
    assert_eq!(v["data"]["run_id"], run_id);
    assert_eq!(v["data"]["already_cancelled"], false);

    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", prefix]));
    assert_eq!(v["data"]["manifest"]["run_id"], run_id);
    assert_eq!(v["data"]["manifest"]["status"], "cancelled");
}

#[test]
fn cancel_ambiguous_prefix_errors_with_candidates() {
    let home = TestHome::new();
    // Two runs created together share the ULID's leading timestamp chars; their
    // longest common prefix is guaranteed to match both (and only both).
    let a = create(&home, "spinoff", "one");
    let b = create(&home, "spinoff", "two");
    let lcp: String = a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x)
        .collect();
    assert!(
        !lcp.is_empty(),
        "ULIDs created together share the timestamp head"
    );

    let (code, v) = run_fail(bin(&home).args(["--output", "json", "run", "cancel", &lcp]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "ambiguous_run_id");
    let candidates = v["error"]["expected"]
        .as_array()
        .expect("expected is array");
    assert!(candidates.iter().any(|c| c == &json!(a)));
    assert!(candidates.iter().any(|c| c == &json!(b)));
}

#[test]
fn cancel_unknown_prefix_returns_run_not_found() {
    let home = TestHome::new();
    let _run_id = create(&home, "spinoff", "present");
    // A well-formed prefix that matches no run → run_not_found (not invalid).
    let (code, v) = run_fail(bin(&home).args(["--output", "json", "run", "cancel", "7zzzzzzzz"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run_not_found");
    assert_eq!(v["error"]["invalid_value"], "7zzzzzzzz");
}

#[test]
fn cancel_impossible_prefix_leading_digit_returns_invalid_run_id() {
    let home = TestHome::new();
    // A valid ULID's first char is bounded to 0..=7 (timestamp range), so an
    // 8-/9-leading prefix can never match any run — it is malformed, not merely
    // absent, and must classify as invalid_run_id (not run_not_found).
    let (code, v) = run_fail(bin(&home).args(["--output", "json", "run", "cancel", "8abc"]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "invalid_run_id");
    assert_eq!(v["error"]["invalid_value"], "8abc");
}

#[test]
fn reattach_spawns_supervisor_and_records_events() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "x");
    let v = run_ok(bin(&home).args(["--output", "json", "run", "reattach", &run_id, "--once"]));
    assert_eq!(v["data"]["action"], "reattached");
    assert!(v["data"]["supervisor_pid"].as_u64().is_some());

    // Give the spawned --once supervisor a beat to write its exit event.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let kinds: Vec<String> = events
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["kind"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(
        kinds.contains(&"supervisor.reattach-requested".to_string()),
        "kinds: {kinds:?}"
    );
    assert!(
        kinds.contains(&"supervisor.reattached".to_string()),
        "kinds: {kinds:?}"
    );
}

#[test]
fn reattach_missing_run_returns_run_not_found() {
    let home = TestHome::new();
    let (code, v) = run_fail(bin(&home).args([
        "--output",
        "json",
        "run",
        "reattach",
        "01jzabsent0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run_not_found");
}

#[test]
fn list_filters_by_kind_and_status() {
    let home = TestHome::new();
    let a = create(&home, "spinoff", "a");
    let _b = create(&home, "orchestrated", "b");

    let v = run_ok(bin(&home).args(["--output", "json", "run", "list", "--kind", "spinoff"]));
    let runs = v["data"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], a);

    // Filter that matches nothing returns an empty list (not an error).
    let v = run_ok(bin(&home).args(["--output", "json", "run", "list", "--status", "done"]));
    assert!(v["data"]["runs"].as_array().unwrap().is_empty());
}

#[test]
fn list_when_root_missing_returns_empty() {
    let home = TestHome::new();
    // No runs created — runs/ dir does not exist yet.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "list"]));
    assert!(v["data"]["runs"].as_array().unwrap().is_empty());
}

// --- `run cancel` terminal-run + convergence semantics ---------------------
//
// These drive a run's state through the sanctioned `event create` write path
// (a temp JSON `--from-file` payload), then exercise `run cancel`.

/// Append one event via `event create`, writing `data` to a temp file the
/// command reads with `--from-file`. The `keep` `TempDir` must outlive the call.
fn event_create(home: &TempDir, run_id: &str, kind: &str, node_id: Option<&str>, data: Value) {
    let keep = TempDir::new().unwrap();
    let f = keep.path().join("data.json");
    std::fs::write(&f, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut cmd = bin(home);
    cmd.args([
        "--output", "json", "event", "create", run_id, "--kind", kind,
    ]);
    if let Some(n) = node_id {
        cmd.args(["--node-id", n]);
    }
    cmd.args(["--from-file", f.to_str().unwrap()]);
    run_ok(&mut cmd);
}

fn add_node(home: &TempDir, run_id: &str, node_id: &str) {
    event_create(
        home,
        run_id,
        "node.created",
        Some(node_id),
        json!({ "kind": "spinoff" }),
    );
}

/// Settle a node via the §7.3-owned `node report` path (`node.report` is not
/// routable through `event create`).
fn node_report(home: &TempDir, run_id: &str, node_id: &str, data: Value) {
    let keep = TempDir::new().unwrap();
    let f = keep.path().join("report.json");
    std::fs::write(&f, serde_json::to_vec(&data).unwrap()).unwrap();
    run_ok(bin(home).args([
        "--output",
        "json",
        "node",
        "report",
        run_id,
        node_id,
        "--from-file",
        f.to_str().unwrap(),
    ]));
}

#[test]
fn cancel_done_run_is_refused_run_already_terminal() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "done-run");
    add_node(&home, &run_id, "n-0001");
    // Settle the node, then the run, to Done.
    node_report(&home, &run_id, "n-0001", json!({ "success": true }));
    event_create(
        &home,
        &run_id,
        "run.status",
        None,
        json!({ "status": "done" }),
    );

    let (code, v) = run_fail(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "run_already_terminal");
    assert_eq!(v["error"]["invalid_value"], "done");
    assert_eq!(
        v["error"]["expected"],
        json!(["running", "pending", "blocked"])
    );

    // The refusal mutated nothing: the run is still Done.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", &run_id]));
    assert_eq!(v["data"]["manifest"]["status"], "done");
}

#[test]
fn cancel_already_cancelled_run_converges_live_node() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "interrupted-cancel");
    add_node(&home, &run_id, "n-0001");
    add_node(&home, &run_id, "n-0002");
    // Simulate an interrupted cancel: run marked cancelled, but n-0001 settled
    // while n-0002 is still live (its node.report never landed).
    node_report(
        &home,
        &run_id,
        "n-0001",
        json!({ "success": false, "cancelled": true, "reason": "stop" }),
    );
    event_create(
        &home,
        &run_id,
        "run.status",
        None,
        json!({ "status": "cancelled" }),
    );

    // Re-cancel converges the straggler and reports it.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], true);
    assert_eq!(
        v["data"]["cancelled_nodes"].as_array().unwrap(),
        &vec![Value::from("n-0002")]
    );
    assert_eq!(
        v["data"]["nodes_already_terminal"].as_array().unwrap(),
        &vec![Value::from("n-0001")]
    );

    // n-0002 is now cancelled on disk.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "show", &run_id]));
    assert_eq!(v["data"]["counts"]["nodes"], 2);
}

#[test]
fn recancel_fully_converged_run_reports_no_new_changes() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "converged");
    add_node(&home, &run_id, "n-0001");

    // First cancel converges everything.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], false);
    assert_eq!(
        v["data"]["cancelled_nodes"].as_array().unwrap(),
        &vec![Value::from("n-0001")]
    );

    // Second cancel: already cancelled, nothing left to converge.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], true);
    assert!(v["data"]["cancelled_nodes"].as_array().unwrap().is_empty());
    assert_eq!(
        v["data"]["nodes_already_terminal"].as_array().unwrap(),
        &vec![Value::from("n-0001")]
    );
}

#[test]
fn cancel_does_not_over_report_already_terminal_node() {
    let home = TestHome::new();
    let run_id = create(&home, "spinoff", "mixed");
    add_node(&home, &run_id, "n-0001");
    add_node(&home, &run_id, "n-0002");
    // n-0001 finishes on its own (Done) before the cancel; a single lock makes
    // the cancel read it as terminal and skip it instead of over-reporting.
    node_report(&home, &run_id, "n-0001", json!({ "success": true }));

    let v = run_ok(bin(&home).args(["--output", "json", "run", "cancel", &run_id]));
    assert_eq!(v["data"]["already_cancelled"], false);
    assert_eq!(
        v["data"]["cancelled_nodes"].as_array().unwrap(),
        &vec![Value::from("n-0002")],
        "the already-Done node must not appear in cancelled_nodes"
    );
    assert_eq!(
        v["data"]["nodes_already_terminal"].as_array().unwrap(),
        &vec![Value::from("n-0001")]
    );
}
