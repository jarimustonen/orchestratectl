//! Integration tests for the `spinoff` subcommand family — `list`,
//! `approve`, `reject`.

use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    // Scrub PATH so a real `issuectl` on the developer's system can't
    // leak into the "missing issuectl" tests. Individual tests that
    // need a fixture issuectl set PATH back explicitly.
    c.env("PATH", "/nonexistent-orchestratectl-test-path");
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
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout is valid JSON");
    // AGENTS-AI-FIRST-CLI §10 success envelope contract.
    assert_eq!(v["schema_version"], 1, "envelope shape: {v}");
    assert!(v.get("data").is_some(), "envelope shape: {v}");
    assert!(v.get("error").is_none(), "envelope shape: {v}");
    // `data` must not double-carry the envelope `warnings` field.
    assert!(
        v["data"].get("warnings").is_none(),
        "data.warnings should live only at envelope level: {v}"
    );
    v
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

fn create_run(home: &TempDir) -> String {
    let v = run_ok(bin(home).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "test-run",
    ]));
    v["data"]["run_id"].as_str().unwrap().to_string()
}

/// Bootstrap a node so the reducer has an anchor for `spinoff.proposed`,
/// then append the proposal via `event create`. Returns the proposal-id.
fn propose(home: &TempDir, run_id: &str, proposal_id: &str, title: &str) {
    // node.created
    let nc = home.path().join(format!("nc-{proposal_id}.json"));
    std::fs::write(
        &nc,
        serde_json::to_vec(&json!({"kind": "spinoff"})).unwrap(),
    )
    .unwrap();
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
    // spinoff.proposed
    let sp = home.path().join(format!("sp-{proposal_id}.json"));
    std::fs::write(
        &sp,
        serde_json::to_vec(&json!({
            "proposal_id": proposal_id,
            "node_id": "n-0001",
            "proposed_title": title,
            "proposed_kind": "spinoff",
            "rationale": "follow-up",
        }))
        .unwrap(),
    )
    .unwrap();
    run_ok(bin(home).args([
        "--output",
        "json",
        "event",
        "create",
        run_id,
        "--kind",
        "spinoff.proposed",
        "--from-file",
        sp.to_str().unwrap(),
    ]));
}

/// Write a stub `issuectl` shell script that prints a fixed JSON
/// payload and returns success. Returns the directory holding the
/// script so the caller can prepend it to PATH.
fn write_stub_issuectl(slug: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let script = dir.path().join("issuectl");
    // `echo` is a /bin/sh builtin so it works even with PATH scrubbed
    // (`cat` would not — see the PATH= override in `bin()`).
    let body = format!(
        "#!/bin/sh\necho '{{\"slug\":\"{slug}\",\"title\":\"x\",\"path\":\"x\",\"dir\":\"x\"}}'\n"
    );
    std::fs::write(&script, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

// ----------------------------- list -----------------------------

#[test]
fn list_empty_when_no_proposals() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let v = run_ok(bin(&home).args(["--output", "json", "spinoff", "list", &run_id]));
    let proposals = v["data"]["proposals"].as_array().unwrap();
    assert!(proposals.is_empty());
}

#[test]
fn list_returns_proposals_with_status_filter() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    propose(&home, &run_id, "s-01bbbbbbbbbbbbbbbbbbbbbbbb", "B");

    let v = run_ok(bin(&home).args(["--output", "json", "spinoff", "list", &run_id]));
    let proposals = v["data"]["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 2);
    for p in proposals {
        assert_eq!(p["status"], "pending");
    }

    // Approve one, then filter.
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "manual-slug",
    ]));
    let v = run_ok(bin(&home).args([
        "--output", "json", "spinoff", "list", &run_id, "--status", "approved",
    ]));
    let approved = v["data"]["proposals"].as_array().unwrap();
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0]["proposal_id"], "s-01aaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(approved[0]["accepted_as_issue_slug"], "manual-slug");
}

#[test]
fn list_unknown_run_id_is_run_not_found() {
    let home = TempDir::new().unwrap();
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "list",
        "01jzabsent0000000000000000",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "run_not_found");
}

#[test]
fn list_rejects_invalid_status_filter() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) = run_fail(bin(&home).args([
        "--output", "json", "spinoff", "list", &run_id, "--status", "bogus",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

// ----------------------------- approve -----------------------------

#[test]
fn approve_writes_event_and_updates_projection_with_manual_slug() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "my-slug",
    ]));
    assert_eq!(v["data"]["issue_slug"], "my-slug");
    assert!(v["data"]["seq"].as_u64().is_some());

    let proj: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("spinoffs")
                .join("s-01aaaaaaaaaaaaaaaaaaaaaaaa.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proj["status"], "approved");
    assert_eq!(proj["accepted_as_issue_slug"], "my-slug");

    let manifest: Value = serde_json::from_slice(
        &std::fs::read(home.path().join("runs").join(&run_id).join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["pending_spinoffs"].as_u64().unwrap(), 0);
}

#[test]
fn approve_is_idempotent_on_reapproval_with_same_slug() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-1",
    ]));
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-1",
    ]));
    assert_eq!(v["data"]["idempotent_replay"], true);
    assert_eq!(v["data"]["issue_slug"], "slug-1");
}

#[test]
fn approve_is_idempotent_on_reapproval_without_slug() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-1",
    ]));
    // Re-approve without specifying --issue-slug returns the recorded slug.
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    assert_eq!(v["data"]["idempotent_replay"], true);
    assert_eq!(v["data"]["issue_slug"], "slug-1");
}

#[test]
fn approve_with_slug_after_approval_without_recorded_slug_errors() {
    // When `issuectl new` is unavailable, the first approve records
    // `accepted_as_issue_slug = null`. A later retry that supplies
    // `--issue-slug` cannot bind it retroactively (B5 contract) — it
    // errors with `proposal_already_approved` and the structured
    // `expected` field is JSON null, not a sentinel string.
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let first = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    assert!(first["data"]["issue_slug"].is_null());

    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-later",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_already_approved");
    assert_eq!(err["error"]["invalid_value"], "slug-later");
    assert!(
        err["error"]["expected"].is_null(),
        "expected should be JSON null when no slug was recorded, got {}",
        err["error"]["expected"]
    );
}

#[test]
fn approve_with_different_slug_is_proposal_already_approved() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-1",
    ]));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "slug-2",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_already_approved");
    assert_eq!(err["error"]["expected"], "slug-1");
    assert_eq!(err["error"]["invalid_value"], "slug-2");
}

#[test]
fn approve_dry_run_does_not_touch_filesystem() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let events_path = home.path().join("runs").join(&run_id).join("events.jsonl");
    let before = std::fs::read(&events_path).unwrap();

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "x",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    let after = std::fs::read(&events_path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn approve_unknown_proposal_is_proposal_not_found() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-99999999999999999999999999",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_not_found");
}

#[test]
fn approve_after_reject_is_proposal_already_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_already_rejected");
}

#[test]
fn approve_without_issue_slug_calls_stub_issuectl() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A title");

    let stub = write_stub_issuectl("auto-materialized-slug");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    cmd.env("ORCHESTRATECTL_HOME", home.path());
    cmd.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    cmd.env("PATH", stub.path());
    cmd.args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]);
    let v = run_ok(&mut cmd);
    assert_eq!(v["data"]["issue_slug"], "auto-materialized-slug");

    let proj: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("spinoffs")
                .join("s-01aaaaaaaaaaaaaaaaaaaaaaaa.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proj["accepted_as_issue_slug"], "auto-materialized-slug");
}

#[test]
fn approve_without_issue_slug_missing_issuectl_succeeds_with_warning() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");

    // bin() already scrubs PATH so issuectl is not found.
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    // Still recorded; no slug; no per-call warning because the
    // "missing on PATH" case is intentionally silent (issuectl is
    // optional).
    assert!(v["data"]["seq"].as_u64().is_some());
    assert!(v["data"]["issue_slug"].is_null());
    let proj: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("spinoffs")
                .join("s-01aaaaaaaaaaaaaaaaaaaaaaaa.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proj["status"], "approved");
}

#[test]
fn approve_issuectl_failure_emits_warning_and_still_records() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");

    // Stub issuectl that exits non-zero.
    let dir = TempDir::new().unwrap();
    let script = dir.path().join("issuectl");
    std::fs::write(&script, "#!/bin/sh\necho boom 1>&2\nexit 17\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    cmd.env("ORCHESTRATECTL_HOME", home.path());
    cmd.env("PATH", dir.path());
    cmd.args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]);
    let v = run_ok(&mut cmd);
    let warnings = v["warnings"].as_array().expect("warnings present");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("issuectl")),
        "expected issuectl warning, got: {warnings:?}"
    );
    assert!(v["data"]["seq"].as_u64().is_some());
    assert!(v["data"]["issue_slug"].is_null());
}

// ----------------------------- reject -----------------------------

#[test]
fn reject_writes_event_and_updates_projection() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "out of scope",
    ]));
    assert!(v["data"]["seq"].as_u64().is_some());

    let proj: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("spinoffs")
                .join("s-01aaaaaaaaaaaaaaaaaaaaaaaa.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proj["status"], "rejected");
    assert_eq!(proj["rejected_reason"], "out of scope");
}

#[test]
fn reject_idempotent_on_matching_reason() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "same",
    ]));
    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "same",
    ]));
    assert_eq!(v["data"]["idempotent_replay"], true);
}

#[test]
fn reject_with_different_reason_is_proposal_already_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "first",
    ]));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "different",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_already_rejected");
}

#[test]
fn reject_dry_run_does_not_touch_filesystem() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let events_path = home.path().join("runs").join(&run_id).join("events.jsonl");
    let before = std::fs::read(&events_path).unwrap();

    let v = run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "x",
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    let after = std::fs::read(&events_path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn reject_after_approve_is_proposal_already_approved() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    run_ok(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "s",
    ]));
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "proposal_already_approved");
}

// ----------------------------- input validation -----------------------------

#[test]
fn approve_rejects_empty_issue_slug() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn approve_rejects_issue_slug_with_uppercase_or_spaces() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "approve",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--issue-slug",
        "My Slug",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn reject_rejects_empty_reason() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "   ",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

#[test]
fn reject_rejects_reason_with_control_chars() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    let (code, err) = run_fail(bin(&home).args([
        "--output",
        "json",
        "spinoff",
        "reject",
        &run_id,
        "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
        "--reason",
        "out\x07of scope",
    ]));
    assert_eq!(code, 1);
    assert_eq!(err["error"]["code"], "invalid_value");
}

// ----------------------------- list ordering -----------------------------

#[test]
fn list_orders_by_proposed_at_desc_with_proposal_id_tiebreaker() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "A");
    propose(&home, &run_id, "s-01bbbbbbbbbbbbbbbbbbbbbbbb", "B");
    propose(&home, &run_id, "s-01ccccccccccccccccccccccccc", "C");

    let v1 = run_ok(bin(&home).args(["--output", "json", "spinoff", "list", &run_id]));
    let v2 = run_ok(bin(&home).args(["--output", "json", "spinoff", "list", &run_id]));
    // Two reads of the same state must produce byte-identical proposal order.
    assert_eq!(v1["data"]["proposals"], v2["data"]["proposals"]);
}

// --------------------------- concurrency -----------------------------------

#[cfg(unix)]
#[test]
fn concurrent_approve_appends_exactly_one_event() {
    use std::thread;
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "race");

    // Two parallel `approve --issue-slug ...` against the same proposal.
    // The flock + decision-enum recheck must collapse the race so only
    // one `spinoff.approved` event lands and the loser sees
    // idempotent_replay.
    let home_path = home.path().to_path_buf();
    let r1 = run_id.clone();
    let r2 = run_id.clone();
    let t1 = thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", &home_path)
            .env("PATH", "/nonexistent-orchestratectl-test-path")
            .args([
                "--output",
                "json",
                "spinoff",
                "approve",
                &r1,
                "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
                "--issue-slug",
                "slug-from-t1",
            ])
            .output()
            .expect("spawn")
    });
    let home_path2 = home.path().to_path_buf();
    let t2 = thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", &home_path2)
            .env("PATH", "/nonexistent-orchestratectl-test-path")
            .args([
                "--output",
                "json",
                "spinoff",
                "approve",
                &r2,
                "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
                "--issue-slug",
                "slug-from-t2",
            ])
            .output()
            .expect("spawn")
    });
    let o1 = t1.join().unwrap();
    let o2 = t2.join().unwrap();

    // Exactly one spinoff.approved event in the log.
    let events =
        std::fs::read_to_string(home.path().join("runs").join(&run_id).join("events.jsonl"))
            .unwrap();
    let count = events
        .lines()
        .filter(|l| l.contains("\"kind\":\"spinoff.approved\""))
        .count();
    assert_eq!(
        count, 1,
        "expected exactly one spinoff.approved event; log:\n{events}"
    );

    // Projection holds whichever slug won.
    let proj: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("spinoffs")
                .join("s-01aaaaaaaaaaaaaaaaaaaaaaaa.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proj["status"], "approved");
    let persisted_slug = proj["accepted_as_issue_slug"].as_str().unwrap();
    assert!(
        persisted_slug == "slug-from-t1" || persisted_slug == "slug-from-t2",
        "persisted slug must be one of the two callers: {persisted_slug}"
    );

    // The caller whose slug won succeeds; the loser surfaces
    // `proposal_already_approved` (per B5: a different `--issue-slug`
    // after approval must not be silently swallowed).
    let (winner, loser, loser_slug) = if persisted_slug == "slug-from-t1" {
        (&o1, &o2, "slug-from-t2")
    } else {
        (&o2, &o1, "slug-from-t1")
    };
    assert!(
        winner.status.success(),
        "winner stderr: {}",
        String::from_utf8_lossy(&winner.stderr)
    );
    assert!(
        !loser.status.success(),
        "loser stdout: {}",
        String::from_utf8_lossy(&loser.stdout)
    );
    let winner_v: Value = serde_json::from_slice(&winner.stdout).unwrap();
    assert_eq!(winner_v["data"]["issue_slug"], persisted_slug);
    let loser_err: Value = serde_json::from_slice(&loser.stderr).unwrap();
    assert_eq!(loser_err["error"]["code"], "proposal_already_approved");
    assert_eq!(loser_err["error"]["expected"], persisted_slug);
    assert_eq!(loser_err["error"]["invalid_value"], loser_slug);
}

#[cfg(unix)]
#[test]
fn concurrent_approve_vs_reject_does_not_lie_about_outcome() {
    use std::thread;
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    propose(&home, &run_id, "s-01aaaaaaaaaaaaaaaaaaaaaaaa", "race2");

    let home_path = home.path().to_path_buf();
    let r1 = run_id.clone();
    let r2 = run_id.clone();
    let t_app = thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", &home_path)
            .env("PATH", "/nonexistent-orchestratectl-test-path")
            .args([
                "--output",
                "json",
                "spinoff",
                "approve",
                &r1,
                "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
                "--issue-slug",
                "winner-app",
            ])
            .output()
            .expect("spawn")
    });
    let home_path2 = home.path().to_path_buf();
    let t_rej = thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", &home_path2)
            .env("PATH", "/nonexistent-orchestratectl-test-path")
            .args([
                "--output",
                "json",
                "spinoff",
                "reject",
                &r2,
                "s-01aaaaaaaaaaaaaaaaaaaaaaaa",
                "--reason",
                "winner-rej",
            ])
            .output()
            .expect("spawn")
    });
    let o_app = t_app.join().unwrap();
    let o_rej = t_rej.join().unwrap();
    // Whoever lost must report a deterministic error (not false success).
    let app_ok = o_app.status.success();
    let rej_ok = o_rej.status.success();
    assert!(
        app_ok ^ rej_ok,
        "exactly one must succeed; app_ok={app_ok}, rej_ok={rej_ok}"
    );

    let loser_stderr = if app_ok { &o_rej.stderr } else { &o_app.stderr };
    let stderr_s = String::from_utf8_lossy(loser_stderr);
    let last = stderr_s
        .lines()
        .last()
        .expect("loser must emit error envelope");
    let v: Value = serde_json::from_str(last).expect("loser error JSON");
    let code = v["error"]["code"].as_str().unwrap();
    assert!(
        code == "proposal_already_approved" || code == "proposal_already_rejected",
        "loser must report a status-conflict error, got: {code}"
    );
}
