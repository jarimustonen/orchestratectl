//! Integration tests for `orchestratectl run merge` (issue
//! `bundle-worktree-merge`). The merge backend is stubbed via `OCTL_MERGE_SH`
//! so the tests exercise orchestratectl's integration — node resolution,
//! source resolution, terminal-report submission, failure handling — without
//! a real git worktree, workmux, or tmux.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

mod common;
use common::TestHome;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    c.env("TMUX_BIN", "/usr/bin/true");
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

fn create_run(home: &TempDir, kind: &str, title: &str) -> String {
    run_ok(bin(home).args([
        "--output", "json", "run", "create", "--kind", kind, "--title", title,
    ]))["data"]["run_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn run_dir(home: &TempDir, run_id: &str) -> std::path::PathBuf {
    home.path().join("runs").join(run_id)
}

/// Forge a `node.created` for `n-0001` carrying a real (existing) worktree
/// path + branch so `run merge` can `cd` into it and resolve the branch.
fn forge_worker_node(home: &TempDir, run_id: &str, kind: &str, worktree: &Path, branch: &str) {
    let node = home.path().join(format!("node-{run_id}.json"));
    std::fs::write(
        &node,
        format!(
            r#"{{"kind":"{kind}","task":"x","worktree_path":"{}","branch":"{branch}","tmux_session":"octl","tmux_window_id":"@42"}}"#,
            worktree.display()
        ),
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
        node.to_str().unwrap(),
    ]));
}

/// Write an executable fake merge backend that records its argv (one line) to
/// `<dir>/merge.log` and exits `code`.
fn fake_merge_sh(dir: &Path, code: i32, stderr: &str) -> std::path::PathBuf {
    let p = dir.join("fake-merge.sh");
    let log = dir.join("merge.log");
    let body = format!(
        "#!/bin/bash\nprintf '%s ' \"$@\" >> '{}'\nprintf '\\n' >> '{}'\n{}\nexit {code}\n",
        log.display(),
        log.display(),
        if stderr.is_empty() {
            String::new()
        } else {
            format!("echo '{stderr}' >&2")
        },
    );
    std::fs::write(&p, body).unwrap();
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&p, perms).unwrap();
    p
}

fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

fn node_reports(events: &Path) -> Vec<Value> {
    read_events(events)
        .into_iter()
        .filter(|v| v["kind"] == "node.report")
        .collect()
}

/// A clean merge: the backend exits 0, and `run merge` appends a terminal
/// `node.report` carrying `via: "explicit-merge"`.
#[test]
fn successful_merge_submits_explicit_merge_report() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-ok");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output", "json", "run", "merge", &run_id, "--source", "main",
    ]));
    assert_eq!(v["data"]["merged"], true);
    assert_eq!(v["data"]["branch"], "wt/test-x");
    assert_eq!(v["data"]["source"], "main");

    // The backend was invoked with the resolved target and branch.
    let argv = std::fs::read_to_string(scratch.path().join("merge.log")).unwrap();
    assert!(
        argv.contains("--target main") && argv.contains("wt/test-x"),
        "merge backend argv was {argv:?}"
    );

    // Exactly one terminal report, stamped with the explicit-merge marker.
    let events = run_dir(&home, &run_id).join("events.jsonl");
    let reports = node_reports(&events);
    assert_eq!(reports.len(), 1, "expected one terminal node.report");
    assert_eq!(reports[0]["data"]["success"], true);
    assert_eq!(reports[0]["data"]["via"], "explicit-merge");
}

/// A merge failure (conflict / dirty tree / lock timeout): the backend exits
/// non-zero, `run merge` surfaces `merge_failed`, and NO terminal report is
/// appended — the node stays live for the agent to recover and retry.
#[test]
fn failed_merge_surfaces_error_and_writes_no_report() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-fail");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 1, "Error: rebase conflict");
    let out = bin(&home)
        .env("OCTL_MERGE_SH", &merge_sh)
        .args(["--output", "json", "run", "merge", &run_id])
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "merge failure must exit non-zero");
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON envelope");
    assert_eq!(err["error"]["code"], "merge_failed");

    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(
        node_reports(&events).len(),
        0,
        "a failed merge must not submit a terminal report"
    );
}

/// `--dry-run` resolves inputs and reports the planned merge without invoking
/// the backend or appending any event.
#[test]
fn dry_run_resolves_without_side_effects() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-dry");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 1, "should never run");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output",
        "json",
        "run",
        "merge",
        &run_id,
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["branch"], "wt/test-x");

    // The backend was never invoked and no report was written.
    assert!(
        !scratch.path().join("merge.log").exists(),
        "dry-run must not invoke the merge backend"
    );
    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(node_reports(&events).len(), 0);
}

/// A run id that names no run surfaces `run_not_found` (not a backend spawn).
#[test]
fn missing_run_is_run_not_found() {
    let home = TestHome::new();
    let out = bin(&home)
        .args([
            "--output",
            "json",
            "run",
            "merge",
            "01jxsnap000000000000000000",
        ])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON envelope");
    assert_eq!(err["error"]["code"], "run_not_found");
}
