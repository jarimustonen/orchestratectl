//! Regression coverage for the raw-git self-merge → death *false-failed*
//! tradeoff (issue `raw-git-selfmerge-false-failed`, epic
//! `lifecycle-architecture-review`).
//!
//! Scenario: a worker hand-merges its branch into source with **raw git** (never
//! `orchestratectl run merge`) and then dies. There is no `merge.started`
//! transaction and no typed `RunMerge` origin, so the crash backstop synthesizes
//! a `failed` report even though the worker's content is already in source. This
//! is the accepted thin-model tradeoff — NOT data loss (the branch/worktree are
//! preserved) but a *false-failed* observability gap.
//!
//! These tests pin the 0.2 behavior:
//!
//! - `run show` **never auto-succeeds** the run — it stays `failed`.
//! - `run show` surfaces a distinct, non-mutating `false_failed` hint that names
//!   the git-verified landing and steers to `run salvage` / `run merge`.
//! - The preserved branch + worktree are left on disk (no destructive teardown;
//!   `run show` reads only).
//!
//! State is seeded directly through the core append path (no live supervisor),
//! against a REAL git repo so the `landed` signal is genuinely git-verified.

use std::path::Path;
use std::process::Command;

use octl_core::{append_and_apply_event, ensure_root, new_run_id, NodeId, RunPaths};
use serde_json::{json, Value};
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c
}

fn node_id() -> NodeId {
    NodeId::parse_str("n-0001").unwrap()
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Build a repo where `wt/raw` was **raw-git ff-merged** into `main` (no
/// `run merge`). Returns `(base_sha, worker_branch)`; the worker's content is now
/// in `main` but nothing recorded a merge.
fn repo_with_raw_selfmerge(repo: &Path) -> (String, String) {
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), "base\n").unwrap();
    git(repo, &["add", "f"]);
    git(repo, &["commit", "-qm", "base"]);
    let base = git(repo, &["rev-parse", "HEAD"]);

    // Worker branch commits real work.
    git(repo, &["checkout", "-q", "-b", "wt/raw"]);
    std::fs::write(repo.join("f"), "base\nwork\n").unwrap();
    git(repo, &["commit", "-qam", "worker change"]);

    // The worker HAND-MERGES into main with raw git — the forbidden path.
    git(repo, &["checkout", "-q", "main"]);
    git(repo, &["merge", "-q", "--ff-only", "wt/raw"]);

    (base, "wt/raw".to_string())
}

/// Seed a run whose worker raw-selfmerged then died: `run.created` recording the
/// repo/branch, a `node.created` for the preserved worktree/branch, a crash-death
/// `worker.exited` (killed by signal), a synthesized `failed` `node.report`
/// (crash backstop, supervisor origin), and the `failed` run status. Returns the
/// `RunPaths`.
fn seed_raw_selfmerge_failed(
    home: &Path,
    run_id: &str,
    repo: &Path,
    base: &str,
    branch: &str,
) -> RunPaths {
    ensure_root(home).unwrap();
    let dir = home.join("runs").join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = RunPaths::new(dir, run_id).unwrap();
    append_and_apply_event(
        &paths,
        "run.created",
        None,
        None,
        json!({
            "kind": "spinoff",
            "lifecycle": "autonomous",
            "title": "raw-selfmerge-death",
            "source_repo": repo.display().to_string(),
            "source_branch": "main",
        }),
    )
    .unwrap();
    append_and_apply_event(
        &paths,
        "node.created",
        Some(&node_id()),
        None,
        json!({
            "kind": "spinoff",
            "worktree_path": repo.display().to_string(),
            "branch": branch,
            "base_sha": base,
        }),
    )
    .unwrap();
    // The worker was hard-killed (signal) — no clean exit, so the crash backstop,
    // not the attention path, governs.
    append_and_apply_event(
        &paths,
        "worker.exited",
        Some(&node_id()),
        None,
        json!({ "signal": 9 }),
    )
    .unwrap();
    // The synthesized crash-backstop failure report: success:false, a supervisor
    // origin, a dead-agent reason. NOT a merge marker. The event `data` IS the
    // report payload (the reducer folds it directly onto `last_report`).
    let mut report = json!({ "success": false, "reason": "agent-died" });
    octl_core::ReportOrigin::Supervisor.stamp(&mut report);
    append_and_apply_event(&paths, "node.report", Some(&node_id()), None, report).unwrap();
    append_and_apply_event(
        &paths,
        "run.status",
        None,
        None,
        json!({ "status": "failed" }),
    )
    .unwrap();
    paths
}

fn run_show_json(home: &TempDir, run_id: &str) -> Value {
    let out = bin(home)
        .args(["--output", "json", "run", "show", run_id])
        .output()
        .expect("spawn run show");
    assert!(
        out.status.success(),
        "run show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("run show json")
}

/// The core regression: a `failed` run whose worker raw-selfmerged then died is
/// surfaced as a *suspected false-failed* by `run show` — git-verified landed,
/// no merge recorded — WITHOUT ever flipping the run to `done`.
#[test]
fn run_show_flags_raw_selfmerge_death_as_false_failed_without_auto_success() {
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let (base, branch) = repo_with_raw_selfmerge(repo.path());
    let run_id = new_run_id();
    seed_raw_selfmerge_failed(home.path(), &run_id, repo.path(), &base, &branch);

    let v = run_show_json(&home, &run_id);
    let d = &v["data"];

    // NO auto-success: the run stays `failed`.
    assert_eq!(
        d["status"], "failed",
        "a raw-selfmerge death must never be auto-promoted to done: {d}"
    );

    // Git CONFIRMS the content is in source (the whole tell).
    assert_eq!(d["landed"], true, "content is git-verified in source: {d}");
    assert_eq!(
        d["landed_method"], "git-verified",
        "the landing must rest on git ground truth, not a report marker: {d}"
    );

    // The distinct false-failed hint is surfaced, pointing at salvage.
    let ff = &d["false_failed"];
    assert!(ff.is_object(), "false_failed block must be present: {d}");
    assert_eq!(
        ff["reason"],
        "branch content is git-verified in source but no `run merge` recorded it (raw-git self-merge?)",
        "false_failed reason: {ff}"
    );
    let hint = ff["resume_hint"].as_str().expect("resume_hint str");
    assert!(
        hint.contains("run salvage") && hint.contains(&run_id),
        "hint must steer to `run salvage <id>`: {hint}"
    );
    assert!(
        hint.contains("run merge"),
        "hint must reinforce finishing through run merge: {hint}"
    );

    // NO destructive teardown: `run show` reads only — the preserved branch and
    // worktree are still on disk.
    assert!(
        Path::new(&repo.path().join(".git")).exists(),
        "worktree/repo must be preserved (run show never tears down)"
    );
    let branch_ref = git(repo.path(), &["rev-parse", "--verify", &branch]);
    assert!(
        !branch_ref.is_empty(),
        "the worker branch must be preserved, not deleted"
    );

    // Text output carries a `false-failed:` line naming salvage.
    let out = bin(&home)
        .args(["--output", "text", "run", "show", &run_id])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        text.lines()
            .any(|l| l.starts_with("false-failed:") && l.contains("run salvage")),
        "text show must carry a false-failed line: {text}"
    );
}

/// Control: a genuinely UNLANDED failed run (worker died before merging anything)
/// must NOT be flagged false-failed — there is no content in source to suspect.
#[test]
fn run_show_does_not_flag_genuinely_unlanded_failed_run() {
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    // Build a repo where wt/raw has an UNMERGED commit (never merged into main).
    let repo_path = repo.path();
    git(repo_path, &["init", "-q", "-b", "main"]);
    git(repo_path, &["config", "user.email", "t@t"]);
    git(repo_path, &["config", "user.name", "t"]);
    std::fs::write(repo_path.join("f"), "base\n").unwrap();
    git(repo_path, &["add", "f"]);
    git(repo_path, &["commit", "-qm", "base"]);
    let base = git(repo_path, &["rev-parse", "HEAD"]);
    git(repo_path, &["checkout", "-q", "-b", "wt/raw"]);
    std::fs::write(repo_path.join("f"), "base\nwork\n").unwrap();
    git(repo_path, &["commit", "-qam", "unmerged work"]);
    git(repo_path, &["checkout", "-q", "main"]);

    let run_id = new_run_id();
    seed_raw_selfmerge_failed(home.path(), &run_id, repo_path, &base, "wt/raw");

    let v = run_show_json(&home, &run_id);
    let d = &v["data"];
    assert_eq!(d["status"], "failed");
    assert_eq!(d["landed"], false, "an unmerged branch is not landed: {d}");
    assert!(
        d.get("false_failed").is_none() || d["false_failed"].is_null(),
        "an honestly-failed unlanded run must not be flagged false-failed: {d}"
    );
}
