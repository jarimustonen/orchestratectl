//! Integration tests for `orchestratectl run wait`.
//!
//! These synthesize terminal manifests directly through the sanctioned write
//! path (`run create --skip-materialize` + `node report` + `event create
//! run.status`) rather than spawning real supervisors — the wait loop is a
//! read-only poll of `manifest.status`, so a live supervisor adds nothing but
//! latency and PTY pressure. No `#[file_serial]` gate is needed (no supervisor
//! is spawned); the `TestHome` fixture still reaps any stray process on drop.

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

/// Run a command expecting a specific non-zero exit code, returning the parsed
/// stdout JSON (the data envelope — `run wait` emits it even on exit 2/3).
fn run_exit(cmd: &mut Command, want: i32) -> Value {
    let out = cmd.output().expect("spawn");
    let code = out.status.code().expect("exit code");
    assert_eq!(
        code,
        want,
        "want exit {want}, got {code}; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is valid JSON")
}

/// Run a command expecting an error envelope on stderr at `want` exit.
fn run_err(cmd: &mut Command, want: i32) -> Value {
    let out = cmd.output().expect("spawn");
    let code = out.status.code().expect("exit code");
    assert_eq!(code, want, "want exit {want}, got {code}");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr has at least one line");
    serde_json::from_str(last).expect("error envelope JSON")
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

/// Append one event via `event create`, writing `data` to a temp file.
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

/// Drive a fresh run to a terminal `manifest.status`, folding the given report
/// into node `n-0001`. `status` is the terminal `run.status` to stamp.
fn settle_run(home: &TempDir, title: &str, status: &str, report: Value) -> String {
    let run_id = create(home, "spinoff", title);
    add_node(home, &run_id, "n-0001");
    node_report(home, &run_id, "n-0001", report);
    event_create(
        home,
        &run_id,
        "run.status",
        None,
        json!({ "status": status }),
    );
    run_id
}

/// A run left at `pending` — created with no node, never settled.
fn pending_run(home: &TempDir, title: &str) -> String {
    create(home, "spinoff", title)
}

#[test]
fn all_happy_path_two_done_runs_exit_zero() {
    let home = TestHome::new();
    let a = settle_run(
        &home,
        "a",
        "done",
        json!({ "success": true, "summary": "did A", "via": "explicit-merge" }),
    );
    let b = settle_run(
        &home,
        "b",
        "done",
        json!({ "success": true, "summary": "did B", "via": "explicit-merge" }),
    );

    // Default condition is --all; both runs are already terminal.
    let v = run_ok(bin(&home).args(["--output", "json", "run", "wait", &a, &b]));
    assert_eq!(v["data"]["condition"], "all");
    let runs = v["data"]["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 2);
    for r in runs {
        assert_eq!(r["status"], "done");
        assert_eq!(r["merged"], true);
        // With no source repo/branch to git-verify against, the `landed` signal
        // falls back to the durable `via: explicit-merge` marker.
        assert_eq!(r["landed"], true);
        assert_eq!(r["landed_method"], "report-marker");
        assert!(r["summary"].as_str().unwrap().starts_with("did "));
        assert!(r.get("error").is_none(), "done run has no error: {r}");
    }
    assert!(v["data"]["waited_ms"].is_number());
}

#[test]
fn any_returns_when_one_of_two_is_terminal() {
    let home = TestHome::new();
    let done = settle_run(
        &home,
        "done",
        "done",
        json!({ "success": true, "summary": "ok" }),
    );
    // The second run never settles; --any must still return promptly.
    let pending = pending_run(&home, "pending");

    let v = run_ok(bin(&home).args(["--output", "json", "run", "wait", &done, &pending, "--any"]));
    assert_eq!(v["data"]["condition"], "any");
    let runs = v["data"]["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 2);
    // The first-listed terminal run is reported done; the other is still pending.
    let by_id = |id: &str| runs.iter().find(|r| r["run_id"] == id).unwrap();
    assert_eq!(by_id(&done)["status"], "done");
    assert_eq!(by_id(&pending)["status"], "pending");
}

#[test]
fn timeout_without_terminal_run_exits_two() {
    let home = TestHome::new();
    let pending = pending_run(&home, "pending");

    // No run will ever settle within the budget → exit 2, with waited_ms ≈ 500.
    let v = run_exit(
        bin(&home).args([
            "--output",
            "json",
            "run",
            "wait",
            &pending,
            "--timeout",
            "500ms",
        ]),
        2,
    );
    assert_eq!(v["data"]["condition"], "all");
    let waited = v["data"]["waited_ms"].as_u64().expect("waited_ms u64");
    assert!(
        (400..=2000).contains(&waited),
        "waited_ms {waited} should be ~500ms (the timeout budget)"
    );
    assert_eq!(v["data"]["runs"][0]["status"], "pending");
}

#[test]
fn fail_on_error_with_failed_run_exits_three() {
    let home = TestHome::new();
    let failed = settle_run(
        &home,
        "failed",
        "failed",
        json!({ "success": false, "summary": "blew up" }),
    );

    // Condition is met (the run is terminal) but it failed → exit 3.
    let v = run_exit(
        bin(&home).args([
            "--output",
            "json",
            "run",
            "wait",
            &failed,
            "--fail-on-error",
        ]),
        3,
    );
    assert_eq!(v["data"]["runs"][0]["status"], "failed");

    // Without --fail-on-error the same terminal state is a plain success (exit 0).
    let v = run_ok(bin(&home).args(["--output", "json", "run", "wait", &failed]));
    assert_eq!(v["data"]["runs"][0]["status"], "failed");
}

#[test]
fn unknown_run_id_exits_one() {
    let home = TestHome::new();
    // Well-formed ULID that names no run on disk.
    let ghost = "01arz3ndektsv4rrffq69g5fav";
    let v = run_err(
        bin(&home).args(["--output", "json", "run", "wait", ghost]),
        1,
    );
    assert_eq!(v["error"]["code"], "unknown_run");
    assert_eq!(v["error"]["invalid_value"], ghost);
}

#[test]
fn malformed_timeout_is_rejected_up_front() {
    let home = TestHome::new();
    let pending = pending_run(&home, "pending");
    let v = run_err(
        bin(&home).args([
            "--output",
            "json",
            "run",
            "wait",
            &pending,
            "--timeout",
            "soon",
        ]),
        1,
    );
    assert_eq!(v["error"]["code"], "invalid_arguments");
}

/// End-to-end proof of the issue's fix: `run wait` reports `landed: true`
/// (git-verified) even after the caller rebases local `main` so the worker's
/// merge is replayed under a new hash — the exact case where
/// `git merge-base --is-ancestor <worker-branch> main` lies.
#[test]
fn wait_reports_landed_git_verified_after_caller_rebase() {
    use std::process::Command;

    fn git(repo: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    // Build a repo whose worker branch merged into main, then rebase local main
    // (replaying the merge under a new hash; the worker branch ref stays put).
    let repo_dir = TempDir::new().unwrap();
    let repo = repo_dir.path();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), "base\n").unwrap();
    git(repo, &["add", "f"]);
    git(repo, &["commit", "-qm", "base"]);
    let base = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["checkout", "-q", "-b", "wt/worker"]);
    std::fs::write(repo.join("f"), "base\nwork\n").unwrap();
    git(repo, &["commit", "-qam", "worker change"]);
    git(repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("g"), "other\n").unwrap();
    git(repo, &["add", "g"]);
    git(repo, &["commit", "-qm", "other session"]);
    let worker_tip = git(repo, &["rev-parse", "wt/worker"]);
    git(repo, &["checkout", "-q", "-b", "replay", &worker_tip]);
    git(repo, &["rebase", "-q", "main"]);
    git(repo, &["checkout", "-q", "main"]);
    git(repo, &["merge", "-q", "--ff-only", "replay"]);
    git(repo, &["branch", "-q", "-D", "replay"]);
    git(repo, &["checkout", "-q", "-b", "tmp", &base]);
    std::fs::write(repo.join("h"), "upstream\n").unwrap();
    git(repo, &["add", "h"]);
    git(repo, &["commit", "-qm", "origin moved"]);
    git(repo, &["checkout", "-q", "main"]);
    git(repo, &["rebase", "-q", "tmp"]);

    // Precondition: the ancestry check the old skill guidance used now lies.
    let is_ancestor = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", "wt/worker", "main"])
        .status()
        .unwrap()
        .success();
    assert!(
        !is_ancestor,
        "the rebase-replay case must make --is-ancestor lie"
    );

    // Wire the run to that repo: source_repo + source_branch on the manifest,
    // branch + base_sha on the node. No explicit-merge marker on the report, so
    // `landed` can only come from git verification.
    let home = TestHome::new();
    let run_id = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "rebase-landed",
        "--source-repo",
        repo.to_str().unwrap(),
        "--source-branch",
        "main",
    ]))["data"]["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    event_create(
        &home,
        &run_id,
        "node.created",
        Some("n-0001"),
        json!({
            "kind": "spinoff",
            "branch": "wt/worker",
            "base_sha": base,
            "worktree_path": repo.to_str().unwrap(),
        }),
    );
    node_report(
        &home,
        &run_id,
        "n-0001",
        json!({ "success": true, "summary": "landed via rebase-replay" }),
    );
    event_create(
        &home,
        &run_id,
        "run.status",
        None,
        json!({ "status": "done" }),
    );

    let v = run_ok(bin(&home).args(["--output", "json", "run", "wait", &run_id]));
    let r = &v["data"]["runs"][0];
    assert_eq!(r["status"], "done");
    assert_eq!(
        r["landed"], true,
        "content is merged → landed must be true despite the rebase: {r}"
    );
    assert_eq!(
        r["landed_method"], "git-verified",
        "the landing is confirmed by patch-id, not the report marker: {r}"
    );
    // No explicit-merge marker → the report-based `merged` flag stays false,
    // proving `landed` is independently git-derived.
    assert_eq!(r["merged"], false);
}

#[test]
fn all_and_any_are_mutually_exclusive() {
    let home = TestHome::new();
    let pending = pending_run(&home, "pending");
    let out = bin(&home)
        .args(["run", "wait", &pending, "--all", "--any"])
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(1),
        "conflicting flags are a usage error"
    );
}
