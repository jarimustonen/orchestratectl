//! Integration tests for the `discussion` subcommand family.

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
        "--output",
        "json",
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
        "--output",
        "json",
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
        "--output",
        "json",
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
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    let v = run_ok(bin(&home).args(["--output", "json", "discussion", "list", &run_id]));
    let arr = v["data"]["discussions"].as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["discussion_id"], "d-abcdefghij");
    assert_eq!(arr[0]["status"], "open");
}

#[test]
fn list_filters_by_status() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    // Resolve it.
    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "keep",
    ]));

    let open = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "list",
        &run_id,
        "--status",
        "open",
    ]));
    assert_eq!(open["data"]["discussions"].as_array().unwrap().len(), 0);

    let resolved = run_ok(bin(&home).args([
        "--output",
        "json",
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

/// A topic carrying a literal newline must not spoof a second physical row
/// in `--format text` list output: the control char is escaped to `\n` so the
/// whole discussion stays on one line.
#[test]
fn list_text_escapes_newline_in_topic() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "line one\nline two\tcol");

    let out = bin(&home)
        .args(["--output", "text", "discussion", "list", &run_id])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit={:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");

    // Exactly one non-empty physical line — the newline did not split the row.
    let rows: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 1, "topic newline spawned extra rows: {rows:?}");
    // The raw control chars are gone; their escapes are present.
    assert!(
        rows[0].contains("line one\\nline two\\tcol"),
        "topic not escaped: {:?}",
        rows[0]
    );
    assert!(
        !rows[0].contains('\t') || rows[0].matches('\t').count() == 4,
        "an embedded tab leaked into the row: {:?}",
        rows[0]
    );
}

#[test]
fn list_unknown_run_is_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "list",
        "01jzabsent0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "run_not_found");
}

#[test]
fn list_empty_when_no_discussions_dir() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let v = run_ok(bin(&home).args(["--output", "json", "discussion", "list", &run_id]));
    assert_eq!(v["data"]["discussions"].as_array().unwrap().len(), 0);
}

// ---------- show ----------

#[test]
fn show_returns_projection() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "show",
        &run_id,
        "d-abcdefghij",
    ]));
    assert_eq!(v["data"]["discussion_id"], "d-abcdefghij");
    assert_eq!(v["data"]["topic"], "first topic");
    assert_eq!(v["data"]["status"], "open");
}

#[test]
fn show_missing_discussion_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "show",
        &run_id,
        "d-missingfff",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_not_found");
}

#[test]
fn show_invalid_id_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) =
        run_fail(bin(&home).args(["--output", "json", "discussion", "show", &run_id, "../etc"]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_id");
}

// ---------- resolve ----------

#[test]
fn resolve_writes_event_and_updates_projection() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--note",
        "decided in standup",
    ]));
    assert!(v["data"]["seq"].as_u64().unwrap() > 0);
    assert_eq!(v["data"]["choice"], "drop");
    assert_eq!(v["data"]["outcome"], "appended");
    assert_eq!(v["data"]["node_id"], "n-0001");

    // The on-disk event should carry the discussion's node_id at the
    // top level so future per-node `event tail` filters work.
    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let resolved_line = events
        .lines()
        .find(|l| l.contains("\"kind\":\"discussion.resolved\""))
        .expect("discussion.resolved event present");
    let ev: Value = serde_json::from_str(resolved_line).unwrap();
    assert_eq!(ev["node_id"], "n-0001");
    assert_eq!(ev["data"]["resolution"], "drop");
    assert_eq!(ev["data"]["note"], "decided in standup");

    // Projection updated.
    let disc_path = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("discussions")
        .join("d-abcdefghij.json");
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
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
    ]));

    let events_before =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
    ]));
    assert_eq!(v["data"]["outcome"], "no-op");
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
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
    ]));

    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
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
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    let events_path = home.path().join("runs").join(&run_id).join("events.jsonl");
    let before = std::fs::read(&events_path).unwrap();

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["outcome"], "dry-run");
    assert_eq!(v["data"]["would_be"], "appended");

    let after = std::fs::read(&events_path).unwrap();
    assert_eq!(before, after, "dry-run must not append to events.jsonl");

    // Projection still open.
    let disc_path = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("discussions")
        .join("d-abcdefghij.json");
    let disc: Value = serde_json::from_slice(&std::fs::read(&disc_path).unwrap()).unwrap();
    assert_eq!(disc["status"], "open");
}

#[test]
fn resolve_empty_choice_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "   ",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn resolve_idempotency_key_same_payload_replays() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    let v1 = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--idempotency-key",
        "k-1",
    ]));
    let seq1 = v1["data"]["seq"].as_u64().unwrap();
    assert_eq!(v1["data"]["outcome"], "appended");

    let v2 = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--idempotency-key",
        "k-1",
    ]));
    assert_eq!(v2["data"]["outcome"], "idempotent-replay");
    assert_eq!(v2["data"]["seq"].as_u64().unwrap(), seq1);

    // Only one discussion.resolved event should be in the log.
    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let count = events
        .lines()
        .filter(|l| l.contains("\"kind\":\"discussion.resolved\""))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn resolve_idempotency_key_different_choice_is_idempotency_conflict() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--idempotency-key",
        "k-1",
    ]));

    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "keep",
        "--idempotency-key",
        "k-1",
    ]));
    // Idempotency layer must fire before the domain status check.
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "idempotency_conflict");
    assert_eq!(err["error"]["expected"]["prior_resolution"], "drop");
}

#[test]
fn resolve_idempotency_key_different_note_is_idempotency_conflict() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--note",
        "first",
        "--idempotency-key",
        "k-1",
    ]));

    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--note",
        "second",
        "--idempotency-key",
        "k-1",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "idempotency_conflict");
}

#[test]
fn resolve_choice_length_capped() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");
    let huge = "x".repeat(2048);
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        &huge,
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn resolve_dry_run_against_resolved_reports_noop() {
    // Dry-run must surface domain state (#3 in the review): a second
    // dry-run with the same choice on a resolved discussion should be
    // a `no-op` preflight, not a falsely-promised `appended`.
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_discussion(&home, &run_id, "d-abcdefghij", "first topic");

    run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
    ]));

    // Same choice → preflight should report no-op, not appended.
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "drop",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["outcome"], "no-op");

    // Different choice → must error out even in dry-run (not silently
    // promise success).
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-abcdefghij",
        "--choice",
        "keep",
        "--dry-run",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_already_resolved");
}

#[test]
fn resolve_missing_discussion_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-missingfff",
        "--choice",
        "drop",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "discussion_not_found");
}
