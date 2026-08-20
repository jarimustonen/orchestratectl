//! Exact worker-worktree ownership discovery (`run show --current`).

use std::path::{Path, PathBuf};
use std::process::Command;

use octl_core::{append_and_apply_event, ensure_root, NodeId, RunPaths};
use serde_json::{json, Value};
use tempfile::TempDir;

const RUN_A: &str = "01m08c08v5jxzfqf3r36n0sgzd";
const RUN_B: &str = "01m08c08v5422jae649kmwewy9";
const OLD_PREFIX: &str = "01m08c08v5";

fn bin(home: &TempDir, cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    command
        .env("ORCHESTRATECTL_HOME", home.path())
        .current_dir(cwd);
    command
}

fn worktree(root: &Path, name: &str, branch: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.join(".git")).unwrap();
    std::fs::write(
        path.join(".git/HEAD"),
        format!("ref: refs/heads/{branch}\n"),
    )
    .unwrap();
    path
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", cwd)
        .status()
        .expect("git is a declared test dependency");
    assert!(status.success(), "git {args:?} failed");
}

/// Create the production linked-worktree shape: each checkout has a `.git`
/// marker file pointing at `<main>/.git/worktrees/<name>`.
fn linked_worktrees(root: &Path, branch_a: &str, branch_b: &str) -> (PathBuf, PathBuf) {
    let main = root.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(&main, &["config", "user.email", "tests@example.invalid"]);
    git(&main, &["config", "user.name", "Ownership Test"]);
    std::fs::write(main.join("seed"), "seed\n").unwrap();
    git(&main, &["add", "seed"]);
    git(&main, &["commit", "-q", "-m", "seed"]);
    git(&main, &["branch", branch_a]);
    git(&main, &["branch", branch_b]);
    let a = root.join("alpha");
    let b = root.join("beta");
    git(
        &main,
        &["worktree", "add", "-q", a.to_str().unwrap(), branch_a],
    );
    git(
        &main,
        &["worktree", "add", "-q", b.to_str().unwrap(), branch_b],
    );
    assert!(a.join(".git").is_file());
    assert!(b.join(".git").is_file());
    (a, b)
}

fn seed_run(home: &Path, run_id: &str, worktree: &Path, branch: &str) {
    ensure_root(home).unwrap();
    let dir = home.join("runs").join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = RunPaths::new(dir, run_id).unwrap();
    append_and_apply_event(
        &paths,
        "run.created",
        None,
        None,
        json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": branch }),
    )
    .unwrap();
    append_and_apply_event(
        &paths,
        "node.created",
        Some(&NodeId::parse_str("n-0001").unwrap()),
        None,
        json!({
            "kind": "spinoff",
            "branch": branch,
            "worktree_path": worktree,
        }),
    )
    .unwrap();
}

fn show_current(home: &TempDir, cwd: &Path) -> std::process::Output {
    bin(home, cwd)
        .args(["run", "show", "--current", "--output", "json"])
        .output()
        .unwrap()
}

fn error_code(output: &std::process::Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    value["error"]["code"].as_str().unwrap().to_string()
}

#[test]
fn each_worktree_resolves_only_its_full_run_id_when_old_prefix_collides() {
    assert_eq!(&RUN_A[..10], OLD_PREFIX);
    assert_eq!(&RUN_B[..10], OLD_PREFIX);

    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let branch_a = format!("wt/{OLD_PREFIX}-alpha");
    let branch_b = format!("wt/{OLD_PREFIX}-beta");
    let (worktree_a, worktree_b) = linked_worktrees(repos.path(), &branch_a, &branch_b);
    seed_run(home.path(), RUN_A, &worktree_a, &branch_a);
    seed_run(home.path(), RUN_B, &worktree_b, &branch_b);

    for (cwd, expected, other) in [(&worktree_a, RUN_A, RUN_B), (&worktree_b, RUN_B, RUN_A)] {
        let output = show_current(&home, cwd);
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["data"]["run_id"], expected);
        assert_ne!(value["data"]["run_id"], other);
    }
}

#[test]
fn legacy_and_entropy_branch_formats_both_resolve_by_exact_ownership() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let legacy_branch = format!("wt/{OLD_PREFIX}-legacy");
    let entropy_id = &RUN_B[RUN_B.len() - 10..];
    let entropy_branch = format!("wt/{entropy_id}-entropy");
    assert!(!RUN_B.starts_with(entropy_id));

    let (legacy_worktree, entropy_worktree) =
        linked_worktrees(repos.path(), &legacy_branch, &entropy_branch);
    seed_run(home.path(), RUN_A, &legacy_worktree, &legacy_branch);
    seed_run(home.path(), RUN_B, &entropy_worktree, &entropy_branch);

    for (cwd, expected) in [(&legacy_worktree, RUN_A), (&entropy_worktree, RUN_B)] {
        let output = show_current(&home, cwd);
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["data"]["run_id"], expected);
    }
}

#[test]
fn duplicate_exact_path_claims_fail_closed() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let branch = format!("wt/{OLD_PREFIX}-alpha");
    let cwd = worktree(repos.path(), "alpha", &branch);
    seed_run(home.path(), RUN_A, &cwd, &branch);
    seed_run(home.path(), RUN_B, &cwd, &branch);

    let output = show_current(&home, &cwd);
    assert!(!output.status.success());
    assert_eq!(error_code(&output), "run_owner_ambiguous");
}

#[test]
fn stale_branch_evidence_fails_closed() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let current_branch = format!("wt/{OLD_PREFIX}-alpha");
    let cwd = worktree(repos.path(), "alpha", &current_branch);
    seed_run(home.path(), RUN_A, &cwd, &format!("wt/{OLD_PREFIX}-stale"));

    let output = show_current(&home, &cwd);
    assert!(!output.status.success());
    assert_eq!(error_code(&output), "run_owner_stale");
}

#[test]
fn missing_and_malformed_evidence_are_distinct_errors() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let branch = format!("wt/{OLD_PREFIX}-alpha");
    let cwd = worktree(repos.path(), "alpha", &branch);

    let missing = show_current(&home, &cwd);
    assert!(!missing.status.success());
    assert_eq!(error_code(&missing), "run_owner_not_found");

    seed_run(home.path(), RUN_A, &cwd, &branch);
    std::fs::write(
        home.path()
            .join("runs")
            .join(RUN_A)
            .join("nodes/n-0001.json"),
        b"{not-json",
    )
    .unwrap();
    let malformed = show_current(&home, &cwd);
    assert!(!malformed.status.success());
    assert_eq!(error_code(&malformed), "run_owner_malformed");
    assert_eq!(malformed.status.code(), Some(1));
}

#[test]
fn show_current_clap_requires_exactly_one_selector() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let branch = format!("wt/{OLD_PREFIX}-alpha");
    let cwd = worktree(repos.path(), "alpha", &branch);

    for (args, expected) in [
        (vec!["run", "show", "--output", "json"], "missing_argument"),
        (
            vec!["run", "show", RUN_A, "--current", "--output", "json"],
            "invalid_arguments",
        ),
    ] {
        let output = bin(&home, &cwd).args(args).output().unwrap();
        assert!(!output.status.success());
        assert_eq!(error_code(&output), expected);
    }
}

#[test]
fn relative_path_and_missing_branch_are_refused_as_unsafe_evidence() {
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let branch = format!("wt/{OLD_PREFIX}-alpha");
    let cwd = worktree(repos.path(), "alpha", &branch);
    seed_run(home.path(), RUN_A, &cwd, &branch);
    let node_path = home
        .path()
        .join("runs")
        .join(RUN_A)
        .join("nodes/n-0001.json");

    let mut node: Value = serde_json::from_slice(&std::fs::read(&node_path).unwrap()).unwrap();
    node["worktree_path"] = json!(".");
    std::fs::write(&node_path, serde_json::to_vec(&node).unwrap()).unwrap();
    let relative = show_current(&home, &cwd);
    assert!(!relative.status.success());
    assert_eq!(error_code(&relative), "run_owner_malformed");

    node["worktree_path"] = json!(cwd);
    node["branch"] = Value::Null;
    std::fs::write(&node_path, serde_json::to_vec(&node).unwrap()).unwrap();
    let branchless = show_current(&home, &cwd);
    assert!(!branchless.status.success());
    assert_eq!(error_code(&branchless), "run_owner_stale");
}
