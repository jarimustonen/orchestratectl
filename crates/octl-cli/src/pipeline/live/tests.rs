//! Deterministic, offline orchestration tests for the live pipeline driver.
//!
//! Every stage is stubbed — a scripted [`SpecProvider`], a scripted
//! [`VerifyProvider`], and a real-git-committing fake [`CodeHarness`] — driven
//! against a real throwaway git repo. No network and no LLM: the *loop* is under
//! test (floor-as-gate, worktree isolation, tier split, teardown), not the model
//! calls. The one live end-to-end test is gated behind `OCTL_PIPELINE_LIVE=1`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use tempfile::TempDir;

use super::providers::{ScriptedSpec, ScriptedVerify};
use super::*;
use crate::harness::{
    CancelToken, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities, HarnessError,
};

/// Run `git` in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Trimmed stdout of a `git` command in `dir`.
fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A fresh repo on `main` with one seed commit.
fn init_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("seed.txt"), "seed\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-qm", "seed"]);
    dir
}

/// A [`CodeHarness`] that actually writes files + commits in the chunk worktree,
/// so the floor's git-diffing and the supervisor-side merge run against real git
/// state (the only thing stubbed is *which* files the "agent" writes).
struct CommitFake {
    files: BTreeMap<String, String>,
}

impl CommitFake {
    fn new(files: &[(&str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }
}

impl CodeHarness for CommitFake {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }

    fn run_chunk(
        &self,
        req: &ChunkRequest,
        _cancel: &CancelToken,
    ) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        for (rel, content) in &self.files {
            let dest = wt.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&dest, content).unwrap();
        }
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "chunk edit"]);
        let head = git_out(wt, &["rev-parse", "HEAD"]);
        let changed: Vec<PathBuf> = self.files.keys().map(PathBuf::from).collect();
        Ok(ChunkResult::committed(head, changed))
    }
}

/// A one-chunk plan value the scripted spec returns. `files` is `files_touched`;
/// `check_run`/`acceptance_run` are shell commands (default `true`).
fn one_chunk_plan(files: &[&str], check_run: &str, acceptance_run: &str) -> serde_json::Value {
    json!({
        // The driver overwrites schema_version/plan_rev/intent_rev/feature/
        // baseline; the spec is trusted only for chunks + acceptance.
        "acceptance": [{"kind": "check", "desc": "feature exists", "run": acceptance_run}],
        "chunks": [{
            "id": "c1",
            "title": "the feature",
            "tier": "code",
            "brief": "implement the feature",
            "files_touched": files,
            "checks": [{"desc": "chunk check", "run": check_run}],
        }],
    })
}

/// A pipeline config over `repo` with trivial (no-op) floor capture commands, so
/// the baseline and current snapshots are empty and the floor is clean unless a
/// gate (file-scope, checks) genuinely fails.
fn config(repo: &Path, workdir: &Path, plan_files: &[&str]) -> PipelineConfig {
    PipelineConfig {
        repo: repo.to_path_buf(),
        intent: "Add a feature file".to_string(),
        source_branch: "main".to_string(),
        files: plan_files.iter().map(PathBuf::from).collect(),
        slug: Some("demo".to_string()),
        test_cmd: "true".to_string(),
        clippy_cmd: "true".to_string(),
        workdir: workdir.to_path_buf(),
        file_scope_slack: 0,
        keep: false,
        chunk_timeout: None,
    }
}

#[test]
fn happy_path_merges_feature_into_source() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "test -f feature.txt",
        "test -f feature.txt",
    ));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.merged);
    assert!(report.final_commit.is_some());
    assert_eq!(report.chunks.len(), 1);
    assert!(report.chunks[0].merged);
    assert_eq!(report.chunks[0].floor_passed, Some(true));
    assert!(report.verify.as_ref().unwrap().passed);

    // The feature really landed on main.
    let main_files = git_out(repo.path(), &["show", "--stat", "main:feature.txt"]);
    assert_eq!(main_files, "hi", "feature.txt content on main");

    // Teardown removed the integration branch (it merged to source).
    assert!(!super::git::branch_exists(repo.path(), "feat/demo"));

    // Provenance: the spec + verify decisions are decider-tier; the chunk merge
    // is coordinator-tier (design §0.2 tier split).
    let spec_dec = report.decisions.iter().find(|d| d.actor == "spec").unwrap();
    assert_eq!(spec_dec.decision_tier, DecisionTier::Decider);
    let merge_dec = report
        .decisions
        .iter()
        .find(|d| d.actor == "supervisor")
        .unwrap();
    assert_eq!(merge_dec.decision_tier, DecisionTier::Coordinator);
}

#[test]
fn floor_blocks_an_out_of_scope_merge() {
    // The "agent" writes an out-of-scope file (secret.txt) not in files_touched.
    // The file-scope gate must fail and the chunk must NOT merge — the floor is
    // the hard gate (design §4/§14).
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n"), ("secret.txt", "leak\n")]);
    let verify = ScriptedVerify::passing();

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");

    assert_eq!(report.status, "chunk_failed", "{report:#?}");
    assert!(!report.merged);
    assert_eq!(report.chunks[0].floor_passed, Some(false));
    assert!(!report.chunks[0].merged);
    // The out-of-scope file kept the feature off main entirely.
    let main_has = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", "main:feature.txt"])
        .status()
        .unwrap()
        .success();
    assert!(!main_has, "a floor-blocked chunk must never reach main");
    // The blocked chunk's branch — which holds the unmerged commit — is
    // preserved (invariant 5). The integration branch itself never received a
    // commit (the first chunk failed before merging), so it is empty and safely
    // torn down; the chunk branch is where the work lives.
    let branch = report.chunks[0].branch_preserved.as_ref().unwrap();
    assert!(super::git::branch_exists(repo.path(), branch));
    assert!(!super::git::branch_exists(repo.path(), "feat/demo"));
}

#[test]
fn verify_failure_blocks_the_merge() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "does not match intent".to_string(),
        findings: vec!["missing edge case".to_string()],
    });

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");

    assert_eq!(report.status, "verify_failed");
    assert!(!report.merged);
    assert!(
        report.chunks[0].merged,
        "the chunk still merged into the integration branch"
    );
    assert!(!report.verify.as_ref().unwrap().passed);
    // Not merged to source.
    let main_has = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", "main:feature.txt"])
        .status()
        .unwrap()
        .success();
    assert!(!main_has);
}

#[test]
fn acceptance_check_failure_blocks_verify() {
    // The chunk merges, but the executable acceptance check fails — the verify
    // verdict must be false (mechanical acceptance ∧ judge), so no merge.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "false"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing(); // judge says ok, but the check fails

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");

    assert_eq!(report.status, "verify_failed");
    let v = report.verify.as_ref().unwrap();
    assert!(!v.acceptance_checks_passed);
    assert!(v.judged_passed);
    assert!(!v.passed);
    assert!(!report.merged);
}

#[test]
fn invalid_plan_is_retried_once_then_fails() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    // Both attempts return a structurally-invalid plan (no chunks).
    let spec = ScriptedSpec::sequence(vec![json!({"acceptance": []}), json!({"acceptance": []})]);
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let err = run_pipeline(&cfg, &spec, &code, &verify).unwrap_err();
    assert!(matches!(err, PipelineError::PlanInvalid(_)), "{err:?}");
    // The failed run tore its integration branch down (nothing merged).
    assert!(!super::git::branch_exists(repo.path(), "feat/demo"));
}

#[test]
fn invalid_then_valid_plan_recovers_on_retry() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::sequence(vec![
        json!({"nonsense": true}), // invalid
        one_chunk_plan(&["feature.txt"], "true", "true"),
    ]);
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");
    assert_eq!(report.status, "merged", "{report:#?}");
}

#[test]
fn refuses_to_reuse_an_existing_integration_branch() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    git(repo.path(), &["branch", "feat/demo"]);
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let err = run_pipeline(&cfg, &spec, &code, &verify).unwrap_err();
    assert!(matches!(err, PipelineError::Setup(_)), "{err:?}");
}

#[test]
fn two_chunk_dag_stacks_and_merges() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["a.txt", "b.txt"]);
    cfg.intent = "two chunk feature".to_string();
    // c2 depends on c1; both write disjoint in-scope files.
    let plan = json!({
        "acceptance": [{"kind": "check", "desc": "both exist", "run": "test -f a.txt && test -f b.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make a",
             "files_touched": ["a.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "make b", "deps": ["c1"],
             "files_touched": ["b.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    // A per-call harness that writes a.txt for c1 and b.txt for c2 by keying on
    // the chunk id in the request.
    struct PerChunk;
    impl CodeHarness for PerChunk {
        fn capabilities(&self) -> HarnessCapabilities {
            HarnessCapabilities {
                can_author_tests: true,
                reports_usage: false,
                honors_file_scope: false,
                runs_checks: false,
            }
        }
        fn run_chunk(
            &self,
            req: &ChunkRequest,
            _cancel: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            let file = if req.chunk_id == "c1" {
                "a.txt"
            } else {
                "b.txt"
            };
            std::fs::write(req.worktree_path.join(file), "x\n").unwrap();
            git(&req.worktree_path, &["add", "-A"]);
            git(&req.worktree_path, &["commit", "-qm", "edit"]);
            let head = git_out(&req.worktree_path, &["rev-parse", "HEAD"]);
            Ok(ChunkResult::committed(head, vec![PathBuf::from(file)]))
        }
    }

    let report = run_pipeline(
        &cfg,
        &ScriptedSpec::new(plan),
        &PerChunk,
        &ScriptedVerify::passing(),
    )
    .expect("pipeline runs");
    assert_eq!(report.status, "merged", "{report:#?}");
    assert_eq!(report.chunks.len(), 2);
    assert!(report.chunks.iter().all(|c| c.merged));
    // Both files on main.
    for f in ["a.txt", "b.txt"] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(["cat-file", "-e", &format!("main:{f}")])
                .status()
                .unwrap()
                .success(),
            "{f} must be on main"
        );
    }
}

#[test]
fn later_chunk_failure_preserves_the_integration_branch() {
    // c1 passes and merges into feat; c2 writes an out-of-scope file and fails
    // the floor. feat now holds c1's merged work that never reached source, so
    // teardown MUST preserve it (invariant 5, source-relative check).
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["a.txt"]);
    let plan = json!({
        "acceptance": [{"kind": "check", "desc": "a", "run": "true"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make a",
             "files_touched": ["a.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "make b", "deps": ["c1"],
             "files_touched": ["b.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    struct PerChunk;
    impl CodeHarness for PerChunk {
        fn capabilities(&self) -> HarnessCapabilities {
            HarnessCapabilities {
                can_author_tests: true,
                reports_usage: false,
                honors_file_scope: false,
                runs_checks: false,
            }
        }
        fn run_chunk(
            &self,
            req: &ChunkRequest,
            _cancel: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            let files: Vec<&str> = if req.chunk_id == "c1" {
                vec!["a.txt"]
            } else {
                // c2 writes b.txt (in scope) AND an out-of-scope stray.
                vec!["b.txt", "stray.txt"]
            };
            for f in &files {
                std::fs::write(req.worktree_path.join(f), "x\n").unwrap();
            }
            git(&req.worktree_path, &["add", "-A"]);
            git(&req.worktree_path, &["commit", "-qm", "edit"]);
            let head = git_out(&req.worktree_path, &["rev-parse", "HEAD"]);
            Ok(ChunkResult::committed(
                head,
                files.iter().map(PathBuf::from).collect(),
            ))
        }
    }

    let report = run_pipeline(
        &cfg,
        &ScriptedSpec::new(plan),
        &PerChunk,
        &ScriptedVerify::passing(),
    )
    .expect("pipeline runs");
    assert_eq!(report.status, "chunk_failed", "{report:#?}");
    assert!(report.chunks[0].merged, "c1 merged");
    assert!(!report.chunks[1].merged, "c2 floor-blocked");
    // feat/demo holds c1 — preserved, not deleted.
    assert!(super::git::branch_exists(repo.path(), "feat/demo"));
    // ...and none of it reached main.
    assert!(!Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", "main:a.txt"])
        .status()
        .unwrap()
        .success());
}

// --- unit tests for the pure helpers ---------------------------------------

#[test]
fn slugify_produces_safe_slugs() {
    assert_eq!(
        slugify("Add CSV export for users!"),
        "add-csv-export-for-users"
    );
    assert_eq!(slugify("   "), "feature");
    assert_eq!(slugify("!!!"), "feature");
    assert_eq!(slugify("Fix\nsecond line"), "fix");
    // Long input is capped and hyphen-trimmed.
    assert!(slugify(&"x ".repeat(100)).len() <= 48);
}

#[test]
fn resolve_intent_reads_file_or_literal() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("intent.md");
    std::fs::write(&f, "from a file").unwrap();
    assert_eq!(resolve_intent(f.to_str().unwrap()).unwrap(), "from a file");
    assert_eq!(
        resolve_intent(&format!("@{}", f.display())).unwrap(),
        "from a file"
    );
    assert_eq!(
        resolve_intent("a literal intent").unwrap(),
        "a literal intent"
    );
    assert!(resolve_intent("   ").is_err());
}

#[test]
fn topo_order_respects_deps() {
    let plan = plan::parse_and_validate_plan(&json!({
        "schema_version": 2, "plan_rev": 1, "intent_rev": 1,
        "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
        "baseline": {"ref": "feat/f@fork", "test_passlist_hash": "h", "clippy_warnings_hash": "h"},
        "acceptance": [{"kind": "check", "desc": "e2e", "run": "true"}],
        "chunks": [
            {"id": "c2", "title": "t", "tier": "code", "brief": "b", "deps": ["c1"],
             "files_touched": ["b.rs"], "checks": [{"desc": "d", "run": "true"}]},
            {"id": "c1", "title": "t", "tier": "code", "brief": "b",
             "files_touched": ["a.rs"], "checks": [{"desc": "d", "run": "true"}]},
        ],
    }))
    .unwrap();
    let order = topo_order(&plan.chunks);
    // c1 (index 1) must come before c2 (index 0).
    let pos_c1 = order
        .iter()
        .position(|&i| plan.chunks[i].id == "c1")
        .unwrap();
    let pos_c2 = order
        .iter()
        .position(|&i| plan.chunks[i].id == "c2")
        .unwrap();
    assert!(pos_c1 < pos_c2);
}

/// The one live end-to-end test: gated behind `OCTL_PIPELINE_LIVE=1` because it
/// invokes real `claude` + `claude-deepseek` agents and merges. Off by default
/// (the deterministic tests above are the always-on gate).
#[test]
fn live_end_to_end_smoke() {
    if std::env::var("OCTL_PIPELINE_LIVE").as_deref() != Ok("1") {
        eprintln!("skipping live pipeline test (set OCTL_PIPELINE_LIVE=1 to run)");
        return;
    }
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["hello.txt"]);
    cfg.slug = None;
    cfg.intent = "Create a file hello.txt containing the text 'hello world'.".to_string();

    let spec = providers::ClaudeSpecProvider;
    let verify = providers::ClaudeVerifyProvider;
    let code = crate::harness::claude::ClaudeHarness::deepseek("flash");
    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("live pipeline runs");
    eprintln!("live report: {report:#?}");
}
