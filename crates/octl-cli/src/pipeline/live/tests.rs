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

use super::breakers::ResourceBudget;
use super::providers::{ScriptedSpec, ScriptedVerify};
use super::*;
use crate::harness::{
    CancelToken, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities, HarnessError, Usage,
};
use crate::pipeline::{Action, Decider, DeciderVerdict, DecisionContext};
use octl_core::plan::Tier;

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
        // The floor's structured capture is fail-closed: it requires a valid
        // cargo `--message-format=json` stream ending in `build-finished`. These
        // stubs emit a minimal empty-but-valid stream (no artifacts ⇒ zero test
        // binaries; no diagnostics ⇒ zero warnings), so both baseline and tip
        // capture as empty and the floor's test/clippy gates pass — the flow
        // tests exercise file-scope / check / breaker behaviour, not lint
        // content. Extra appended flags (`--no-run`, `--message-format=json`)
        // are ignored by `printf`.
        test_cmd: r#"printf '{"reason":"build-finished","success":true}\n'"#.to_string(),
        clippy_cmd: r#"printf '{"reason":"build-finished","success":true}\n'"#.to_string(),
        workdir: workdir.to_path_buf(),
        file_scope_slack: 0,
        keep: false,
        chunk_timeout: None,
        // Default the fix loop OFF so the pre-loop tests keep asserting the
        // first-failure-is-terminal behaviour; fix-loop tests opt in explicitly.
        fix_loop: super::fixloop::FixLoopConfig::OFF,
        // Default every resource breaker OFF so the pre-breaker tests are unchanged;
        // breaker tests opt in with a tight ceiling explicitly.
        budget: super::breakers::ResourceBudget::UNLIMITED,
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

    assert_eq!(report.status, "chunk_floor_blocked", "{report:#?}");
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
        disposition: providers::VerifyDisposition::Fix,
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

/// A structurally-valid chunk missing the top-level `acceptance` array — the
/// exact shape the first live run produced (`missing field acceptance`).
fn plan_missing_acceptance(files: &[&str]) -> serde_json::Value {
    json!({
        "chunks": [{
            "id": "c1", "title": "t", "tier": "code", "brief": "b",
            "files_touched": files,
            "checks": [{"desc": "d", "run": "true"}],
        }],
    })
}

#[test]
fn persistently_invalid_plan_fails_with_raw_persisted_and_error_surfaced() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    // Every attempt omits the required `acceptance` array (the observed bug).
    let spec = ScriptedSpec::sequence(vec![
        plan_missing_acceptance(&["feature.txt"]),
        plan_missing_acceptance(&["feature.txt"]),
    ]);
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let err = run_pipeline(&cfg, &spec, &code, &verify).unwrap_err();
    // The last validator message is surfaced (not swallowed into "attempt N").
    match &err {
        PipelineError::PlanInvalid(msg) => {
            assert!(
                msg.contains("acceptance"),
                "validator error not surfaced: {msg}"
            );
            assert!(
                msg.contains("plan.invalid.json"),
                "persisted path not named: {msg}"
            );
        }
        other => panic!("expected PlanInvalid, got {other:?}"),
    }
    // The repair loop actually re-prompted once, carrying the error forward.
    let calls = spec.repair_calls();
    assert_eq!(calls.len(), 1, "expected exactly one repair re-prompt");
    assert!(
        calls[0].1.contains("acceptance"),
        "repair was not fed the validator error: {}",
        calls[0].1
    );
    // The raw invalid plan the model produced was persisted for inspection.
    let persisted = workdir.path().join("plan.invalid.json");
    assert!(persisted.is_file(), "invalid plan not persisted");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&persisted).unwrap()).unwrap();
    assert!(
        saved.get("acceptance").is_none(),
        "persisted plan should be the raw invalid one"
    );
    assert!(saved.get("chunks").is_some());
    // The failed run tore its integration branch down (nothing merged).
    assert!(!super::git::branch_exists(repo.path(), "feat/demo"));
}

#[test]
fn repair_loop_feeds_validator_error_back_and_succeeds() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    // Invalid first (missing acceptance), corrected on the repair re-prompt.
    let spec = ScriptedSpec::sequence(vec![
        plan_missing_acceptance(&["feature.txt"]),
        one_chunk_plan(&["feature.txt"], "true", "true"),
    ]);
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let report = run_pipeline(&cfg, &spec, &code, &verify).expect("pipeline runs");
    assert_eq!(report.status, "merged", "{report:#?}");

    // The loop fed the exact validator error (and the invalid JSON) back to the
    // model on the repair attempt — not a blind re-produce.
    let calls = spec.repair_calls();
    assert_eq!(calls.len(), 1, "expected exactly one repair re-prompt");
    let (invalid, error) = &calls[0];
    assert!(error.contains("acceptance"), "error not fed back: {error}");
    assert!(
        invalid.get("acceptance").is_none() && invalid.get("chunks").is_some(),
        "the invalid JSON produced was not fed back: {invalid}"
    );
    // A recovered run must NOT leave a stray invalid-plan artifact behind.
    assert!(!workdir.path().join("plan.invalid.json").exists());
}

#[test]
fn repair_call_failure_persists_the_prior_invalid_plan() {
    // attempt 0 produces an invalid plan; the repair re-prompt itself fails
    // (spawn/timeout). The prior invalid plan must still be persisted for
    // inspection rather than lost behind the transport error.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::sequence_then_error(vec![plan_missing_acceptance(&["feature.txt"])]);
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let verify = ScriptedVerify::passing();

    let err = run_pipeline(&cfg, &spec, &code, &verify).unwrap_err();
    // The surfaced error is the spec transport failure (repair could not run).
    assert!(matches!(err, PipelineError::Spec(_)), "{err:?}");
    // But the invalid plan from attempt 0 was persisted anyway.
    let persisted = workdir.path().join("plan.invalid.json");
    assert!(
        persisted.is_file(),
        "prior invalid plan not persisted on repair failure"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&persisted).unwrap()).unwrap();
    assert!(saved.get("acceptance").is_none() && saved.get("chunks").is_some());
    // The repair path was reached (fed the error) before it failed.
    assert_eq!(spec.repair_calls().len(), 1);
}

#[test]
fn invalid_then_valid_plan_recovers_on_repair() {
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
    assert_eq!(report.status, "chunk_floor_blocked", "{report:#?}");
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

#[test]
fn lying_harness_no_commit_is_blocked_not_merged() {
    // The harness claims `Committed` but never advances HEAD (it left the real
    // work uncommitted, or fabricated the oid). The driver must catch this BEFORE
    // the floor and never merge — otherwise a no-op masquerades as a merged chunk.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let verify = ScriptedVerify::passing();

    struct Liar;
    impl CodeHarness for Liar {
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
            // Write an UNCOMMITTED file, then claim a committed oid == base.
            std::fs::write(req.worktree_path.join("feature.txt"), "hi\n").unwrap();
            Ok(ChunkResult::committed(
                req.base_commit.clone(),
                vec![PathBuf::from("feature.txt")],
            ))
        }
    }

    let report = run_pipeline(&cfg, &spec, &Liar, &verify).expect("pipeline runs");
    assert_eq!(report.status, "chunk_failed", "{report:#?}");
    assert!(!report.merged);
    assert!(!report.chunks[0].merged);
    // Nothing reached main.
    assert!(!Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", "main:feature.txt"])
        .status()
        .unwrap()
        .success());
}

#[test]
fn empty_commit_is_blocked_not_merged() {
    // The harness makes a real (but EMPTY) commit — HEAD advances, diff is empty.
    // An empty diff trivially satisfies file-scope, so without the non-empty-diff
    // guard it would sail through. It must be blocked instead.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let verify = ScriptedVerify::passing();

    struct EmptyCommitter;
    impl CodeHarness for EmptyCommitter {
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
            git(
                &req.worktree_path,
                &["commit", "-q", "--allow-empty", "-m", "empty"],
            );
            let head = git_out(&req.worktree_path, &["rev-parse", "HEAD"]);
            Ok(ChunkResult::committed(head, vec![]))
        }
    }

    let report = run_pipeline(&cfg, &spec, &EmptyCommitter, &verify).expect("pipeline runs");
    assert_eq!(report.status, "chunk_failed", "{report:#?}");
    assert!(!report.merged);
}

// --- fix-loop tests (design §7/§8/§9) --------------------------------------

use super::fixloop::FixLoopConfig;
use std::sync::atomic::{AtomicU32, Ordering};

/// A harness that writes an out-of-scope `stray.txt` (floor-blocking) ONLY on the
/// first attempt (`attempt_id == "a1"`), then behaves on later attempts — so a
/// `RE_CODE` re-brief can un-block it.
struct StrayOnFirstAttempt;
impl CodeHarness for StrayOnFirstAttempt {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        std::fs::write(wt.join("feature.txt"), "hi\n").unwrap();
        let mut changed = vec![PathBuf::from("feature.txt")];
        if req.attempt_id == "a1" {
            std::fs::write(wt.join("stray.txt"), "leak\n").unwrap();
            changed.push(PathBuf::from("stray.txt"));
        }
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            changed,
        ))
    }
}

/// A harness that always writes `stray.txt` — a persistent floor failure no
/// re-code can fix, for exercising the circuit-breaker.
struct AlwaysStray;
impl CodeHarness for AlwaysStray {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        std::fs::write(wt.join("feature.txt"), "hi\n").unwrap();
        std::fs::write(wt.join("stray.txt"), "leak\n").unwrap();
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            vec![PathBuf::from("feature.txt"), PathBuf::from("stray.txt")],
        ))
    }
}

/// A harness that writes a fresh, ever-changing `feature.txt` each call, so a
/// re-code (which forks off the current tip that already holds the prior content)
/// always produces a non-empty diff. In-scope; floor always green.
struct IncrementingFeature {
    calls: AtomicU32,
}
impl IncrementingFeature {
    fn new() -> Self {
        Self {
            calls: AtomicU32::new(0),
        }
    }
}
impl CodeHarness for IncrementingFeature {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let wt = &req.worktree_path;
        std::fs::write(wt.join("feature.txt"), format!("version {n}\n")).unwrap();
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            vec![PathBuf::from("feature.txt")],
        ))
    }
}

#[test]
fn floor_blocked_chunk_recodes_then_merges() {
    // Design §8: a floor-blocked chunk is re-briefed with its findings and
    // re-run; it MUST re-pass the floor before it can merge. The first attempt
    // writes an out-of-scope file (floor block); the RE_CODE attempt does not,
    // so the chunk reaches merge.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));

    let report = run_pipeline(
        &cfg,
        &spec,
        &StrayOnFirstAttempt,
        &ScriptedVerify::passing(),
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.merged);
    assert_eq!(report.recode_count, 1, "exactly one RE_CODE happened");
    assert!(report.chunks[0].merged);
    assert_eq!(report.chunks[0].floor_passed, Some(true));
    // The feature landed on main; the stray never did.
    assert!(main_has(&repo, "feature.txt"));
    assert!(!main_has(&repo, "stray.txt"));
    // A RE_CODE_CHUNK decision was recorded (T4 primitive), coordinator-tier.
    let recode = report
        .decisions
        .iter()
        .find(|d| d.reason.contains("re-code chunk"))
        .expect("a re-code decision is recorded");
    assert_eq!(recode.decision_tier, DecisionTier::Coordinator);
}

#[test]
fn persistent_floor_failure_trips_the_circuit_breaker() {
    // Design §9: a chunk that keeps failing the floor must terminate on the
    // repeated-failure breaker, not loop forever.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 2,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));

    let report =
        run_pipeline(&cfg, &spec, &AlwaysStray, &ScriptedVerify::passing()).expect("pipeline runs");

    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(!report.merged);
    assert!(report.circuit_breaker.is_some(), "breaker reason recorded");
    assert_eq!(report.recode_count, 2, "both re-codes were attempted");
    // The last failing attempt's branch is preserved (invariant 5).
    let branch = report.chunks[0].branch_preserved.as_ref().unwrap();
    assert!(super::git::branch_exists(repo.path(), branch));
    assert!(!main_has(&repo, "feature.txt"));
}

#[test]
fn verify_failure_recodes_then_merges() {
    // Design §8: a failed verify feeds RE_CODE_CHUNK; the re-coded chunk MUST
    // re-verify before close. Verify fails once (FIX), passes after the re-code.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "not yet".to_string(),
            findings: vec!["needs the greeting".to_string()],
            disposition: providers::VerifyDisposition::FixChunks {
                chunk_ids: vec!["c1".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "matches intent".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

    let report =
        run_pipeline(&cfg, &spec, &IncrementingFeature::new(), &verify).expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.merged);
    assert_eq!(report.recode_count, 1, "verify FIX drove one re-code");
    assert!(report.verify.as_ref().unwrap().passed);
    assert!(main_has(&repo, "feature.txt"));
}

#[test]
fn verify_fix_loop_exhaustion_trips_the_breaker() {
    // Design §9: verify that never passes must terminate on the fix-iteration
    // breaker, not loop forever.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 1,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "never happy".to_string(),
        findings: vec!["still wrong".to_string()],
        disposition: providers::VerifyDisposition::Fix,
    });

    let report =
        run_pipeline(&cfg, &spec, &IncrementingFeature::new(), &verify).expect("pipeline runs");

    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(!report.merged);
    assert!(report.circuit_breaker.is_some());
}

#[test]
fn spec_flaw_triggers_respec_then_merges() {
    // Design §7: a SPEC-FLAW verdict emits TRIGGER_RE_SPEC — a new plan.v2 whose
    // DAG-diff reverts the flagged chunk to Pending; the re-coded chunk then
    // re-verifies clean and the feature merges.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 1,
        max_promotions: 0,
    };
    // v1 then v2 (a materially-different brief so the plan revision is distinct).
    let v2 = json!({
        "acceptance": [{"kind": "check", "desc": "exists", "run": "test -f feature.txt"}],
        "chunks": [{
            "id": "c1", "title": "the feature", "tier": "code",
            "brief": "implement the feature CORRECTLY this time",
            "files_touched": ["feature.txt"],
            "checks": [{"desc": "chunk check", "run": "true"}],
        }],
    });
    let spec = ScriptedSpec::sequence(vec![
        one_chunk_plan(&["feature.txt"], "true", "test -f feature.txt"),
        v2,
    ]);
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "the plan cannot meet intent".to_string(),
            findings: vec!["wrong approach".to_string()],
            disposition: providers::VerifyDisposition::SpecFlaw {
                reason: "the plan cannot meet intent".to_string(),
                chunk_ids: vec!["c1".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "now matches".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

    let report =
        run_pipeline(&cfg, &spec, &IncrementingFeature::new(), &verify).expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.merged);
    assert_eq!(report.respec_count, 1, "one re-spec happened");
    assert_eq!(report.plan_rev, 2, "plan advanced to v2");
    // The re-spec fed the flaw reason forward to the spec provider.
    let calls = spec.respec_calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].1.contains("cannot meet intent"),
        "reason fed: {}",
        calls[0].1
    );
    // A TRIGGER_RE_SPEC decision was recorded (T4 primitive), decider-tier.
    let respec = report
        .decisions
        .iter()
        .find(|d| d.reason.contains("re-spec to plan.v2"))
        .expect("a re-spec decision is recorded");
    assert_eq!(respec.decision_tier, DecisionTier::Decider);
    // plan.v2.json was persisted for audit.
    assert!(workdir.path().join("plan.v2.json").is_file());
    assert!(main_has(&repo, "feature.txt"));
}

#[test]
fn verify_fix_with_only_unknown_chunk_ids_does_not_blast_all_chunks() {
    // A FixChunks verdict that names only a hallucinated chunk id must NOT fall
    // back to re-coding every merged chunk — it resolves to no target and the run
    // ends as a plain verify failure (post-review hardening of resolve_fix_targets).
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "nope".to_string(),
        findings: vec!["x".to_string()],
        disposition: providers::VerifyDisposition::FixChunks {
            chunk_ids: vec!["ghost-chunk".to_string()],
        },
    });

    let report =
        run_pipeline(&cfg, &spec, &IncrementingFeature::new(), &verify).expect("pipeline runs");

    assert_eq!(report.status, "verify_failed", "{report:#?}");
    assert_eq!(report.recode_count, 0, "no chunk should be re-coded");
    assert!(!report.merged);
}

#[test]
fn acceptance_check_failure_recodes_with_the_check_as_a_finding() {
    // Judge passes but an executable acceptance check fails → the disposition is a
    // bare Fix, and the failed check description is fed back as a finding so the
    // re-code has context (post-review: acceptance failures reach the re-brief).
    // The acceptance check `test -f marker.txt` fails until the harness writes it.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt", "marker.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt", "marker.txt"],
        "true",
        "test -f marker.txt",
    ));

    // First harness call writes only feature.txt (acceptance fails); the re-code
    // writes marker.txt too (acceptance passes).
    struct MarkerOnRecode {
        calls: AtomicU32,
    }
    impl CodeHarness for MarkerOnRecode {
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
            _c: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let wt = &req.worktree_path;
            std::fs::write(wt.join("feature.txt"), format!("v{n}\n")).unwrap();
            let mut changed = vec![PathBuf::from("feature.txt")];
            if n >= 2 {
                std::fs::write(wt.join("marker.txt"), "m\n").unwrap();
                changed.push(PathBuf::from("marker.txt"));
            }
            git(wt, &["add", "-A"]);
            git(wt, &["commit", "-qm", "edit"]);
            Ok(ChunkResult::committed(
                git_out(wt, &["rev-parse", "HEAD"]),
                changed,
            ))
        }
    }

    // Judge always passes; only the acceptance check drives the failure→fix.
    let verify = ScriptedVerify::passing();
    let report = run_pipeline(
        &cfg,
        &spec,
        &MarkerOnRecode {
            calls: AtomicU32::new(0),
        },
        &verify,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert_eq!(
        report.recode_count, 1,
        "the acceptance failure drove one re-code"
    );
    assert!(main_has(&repo, "marker.txt"));
}

#[test]
fn recode_rebrief_carries_the_prior_failing_diff() {
    // Item 3 (re-code amnesia): the failed attempt's worktree is torn down before
    // the retry, but its committed diff is serialized into the re-brief so the model
    // does not lose the failing work it just produced.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));

    // Records the brief handed to each attempt; the first attempt strays (floor
    // block), the re-code stays in scope and merges.
    struct BriefRecorder {
        briefs: std::sync::Mutex<Vec<String>>,
    }
    impl CodeHarness for BriefRecorder {
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
            _c: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            self.briefs.lock().unwrap().push(req.brief.clone());
            let wt = &req.worktree_path;
            std::fs::write(wt.join("feature.txt"), "FAILING_CONTENT_MARKER\n").unwrap();
            let mut changed = vec![PathBuf::from("feature.txt")];
            if req.attempt_id == "a1" {
                std::fs::write(wt.join("stray.txt"), "leak\n").unwrap();
                changed.push(PathBuf::from("stray.txt"));
            }
            git(wt, &["add", "-A"]);
            git(wt, &["commit", "-qm", "edit"]);
            Ok(ChunkResult::committed(
                git_out(wt, &["rev-parse", "HEAD"]),
                changed,
            ))
        }
    }

    let harness = BriefRecorder {
        briefs: std::sync::Mutex::new(Vec::new()),
    };
    let report =
        run_pipeline(&cfg, &spec, &harness, &ScriptedVerify::passing()).expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    let briefs = harness.briefs.into_inner().unwrap();
    assert_eq!(briefs.len(), 2, "one initial attempt + one re-code");
    // The initial brief is the bare chunk brief — no prior-attempt section.
    assert!(
        !briefs[0].contains("previous attempt's diff"),
        "{}",
        briefs[0]
    );
    // The re-code brief carries the failing attempt's diff as fenced DATA.
    assert!(
        briefs[1].contains("previous attempt's diff"),
        "re-brief missing the diff section: {}",
        briefs[1]
    );
    assert!(briefs[1].contains("```diff"), "{}", briefs[1]);
    // The exact content the failed attempt committed is present in the carried diff.
    assert!(
        briefs[1].contains("FAILING_CONTENT_MARKER"),
        "re-brief lost the failing diff content: {}",
        briefs[1]
    );
    // And the out-of-scope stray that caused the block is visible in the diff too.
    assert!(briefs[1].contains("stray.txt"), "{}", briefs[1]);
}

/// A harness whose per-call floor outcome is scripted: on the `fail_on` (0-based)
/// calls it also writes an out-of-scope `stray.txt` (floor block); otherwise it
/// stays in scope. Every call writes a UNIQUE `feature.txt` content so a re-code
/// forking off an already-populated tip still produces a non-empty diff.
struct ScriptedFloor {
    calls: AtomicU32,
    fail_on: Vec<u32>,
}
impl CodeHarness for ScriptedFloor {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let wt = &req.worktree_path;
        std::fs::write(wt.join("feature.txt"), format!("version {n}\n")).unwrap();
        let mut changed = vec![PathBuf::from("feature.txt")];
        if self.fail_on.contains(&n) {
            std::fs::write(wt.join("stray.txt"), format!("leak {n}\n")).unwrap();
            changed.push(PathBuf::from("stray.txt"));
        }
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            changed,
        ))
    }
}

#[test]
fn cumulative_recode_budget_caps_across_verify_iterations() {
    // Item 2: `max_recode_per_chunk` must bound a chunk's TOTAL floor re-codes
    // across code-stage visits within one plan revision, not reset each visit.
    // Visit 1 spends the chunk's single floor re-code (call 0 strays → re-code →
    // call 1 clean → merge). Verify then fails once (FIX), reverting the chunk for a
    // second code-stage visit whose first attempt (call 2) strays again. With a
    // per-visit reset the chunk would get a fresh re-code and eventually merge; with
    // the cumulative budget the floor re-code is DENIED and the run trips the
    // breaker — the nominal bound of 1 is respected across the two visits.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    // Verify fails once (FIX) to force a second code-stage visit, then would pass.
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "not yet".to_string(),
            findings: vec!["needs work".to_string()],
            disposition: providers::VerifyDisposition::FixChunks {
                chunk_ids: vec!["c1".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "ok".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);
    let harness = ScriptedFloor {
        calls: AtomicU32::new(0),
        fail_on: vec![0, 2], // stray on visit-1 attempt-1 and visit-2 attempt-1
    };

    let report = run_pipeline(&cfg, &spec, &harness, &verify).expect("pipeline runs");

    assert_eq!(
        report.status, "circuit_breaker",
        "cumulative re-code budget must deny the second visit's floor re-code: {report:#?}"
    );
    assert!(!report.merged);
    // Exactly the visit-1 floor re-code plus the one verify-FIX re-brief happened;
    // the visit-2 floor re-code was denied by the cumulative budget (no 3rd re-code).
    assert_eq!(
        report.recode_count, 2,
        "the cumulative budget capped the floor re-codes across visits"
    );
}

#[test]
fn respec_that_removes_a_chunk_rolls_its_code_off_feat() {
    // Item 1 (provenance-aware rollback): a re-spec that DROPS a chunk rebuilds the
    // integration branch from the fork replaying only the kept chunks, so the
    // removed chunk's code is gone from feat instead of stranded there. Without the
    // rollback the removed b.txt would linger and even trip the feature floor as an
    // out-of-scope file; with it, only the kept a.txt reaches source.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["a.txt", "b.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 1,
        max_promotions: 0,
    };
    // v1 has c1 (a.txt) + c2 (b.txt); v2 drops c2, keeping only c1.
    let v1 = json!({
        "acceptance": [{"kind": "check", "desc": "a", "run": "test -f a.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make a",
             "files_touched": ["a.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "make b", "deps": ["c1"],
             "files_touched": ["b.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    let v2 = json!({
        "acceptance": [{"kind": "check", "desc": "a", "run": "test -f a.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make a",
             "files_touched": ["a.txt"], "checks": [{"desc": "a", "run": "true"}]},
        ],
    });
    let spec = ScriptedSpec::sequence(vec![v1, v2]);
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "c2 is wrong; drop it".to_string(),
            findings: vec!["remove b".to_string()],
            disposition: providers::VerifyDisposition::SpecFlaw {
                reason: "c2 is wrong; drop it".to_string(),
                chunk_ids: vec!["c2".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "now matches".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

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

    let report = run_pipeline(&cfg, &spec, &PerChunk, &verify).expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert_eq!(report.plan_rev, 2, "plan advanced to v2");
    assert_eq!(report.respec_count, 1, "one re-spec happened");
    // The kept chunk's code is on main; the removed chunk's code was rolled back.
    assert!(
        main_has(&repo, "a.txt"),
        "kept chunk c1's file must reach main"
    );
    assert!(
        !main_has(&repo, "b.txt"),
        "removed chunk c2's code must be rolled off feat, not stranded on it"
    );
}

/// A per-chunk harness for the two-chunk rollback tests: `c1` writes `shared.txt`
/// = "base\n"; `c2` writes "base\nextra\n" (a modification of c1's content, so
/// replaying c2 onto a tree WITHOUT c1's `shared.txt` conflicts). Deterministic and
/// stateless — each call rewrites its chunk's whole file, so a re-run is clean.
struct SharedFileChunks;
impl CodeHarness for SharedFileChunks {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        let content = if req.chunk_id == "c1" {
            "base\n"
        } else {
            "base\nextra\n"
        };
        std::fs::write(wt.join("shared.txt"), content).unwrap();
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            vec![PathBuf::from("shared.txt")],
        ))
    }
}

#[test]
fn verify_fix_rollback_reverts_transitive_dependents() {
    // Review fix (dependency-aware rollback): when verify-FIX targets c1 and c2
    // depends on c1 (and modifies the same file), the rollback must revert BOTH —
    // keeping c2 while dropping c1 would replay c2 onto a tree missing c1's content
    // and conflict. With the dependent closure, both revert, both re-run, and the
    // feature converges.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["shared.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let plan = json!({
        "acceptance": [{"kind": "check", "desc": "exists", "run": "test -f shared.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make base",
             "files_touched": ["shared.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "append", "deps": ["c1"],
             "files_touched": ["shared.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "fix c1".to_string(),
            findings: vec!["c1 wrong".to_string()],
            disposition: providers::VerifyDisposition::FixChunks {
                chunk_ids: vec!["c1".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "ok".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

    let report = run_pipeline(&cfg, &ScriptedSpec::new(plan), &SharedFileChunks, &verify)
        .expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.merged);
    // Only c1 was a re-code target; c2 was reverted as a dependent (no re-code
    // decision of its own), so exactly one RE_CODE was recorded.
    assert_eq!(report.recode_count, 1);
    // The reconverged feature is on main with c2's final content.
    assert_eq!(
        git_out(repo.path(), &["show", "main:shared.txt"]),
        "base\nextra"
    );
}

#[test]
fn rollback_conflict_yields_a_terminal_report_naming_the_chunk() {
    // Item B (graceful rollback_conflict report) + transactional rollback: if a kept
    // chunk cannot be replayed onto the rebuilt fork (here c2 modifies c1's file but
    // does NOT declare the dependency, so the closure can't catch it), the rollback
    // must (1) restore the integration branch to its intact pre-rollback tip — never
    // leave a half-rebuilt branch, so invariant-5 preservation keeps the REAL work —
    // and (2) yield a terminal `rollback_conflict` PipelineReport naming the chunk
    // that failed to replay, rather than a bare PipelineError::Git.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["shared.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    // c2 modifies shared.txt but declares NO dep on c1 — the closure keeps c2 while
    // c1 is dropped, and the replay of c2 onto the fork conflicts.
    let plan = json!({
        "acceptance": [{"kind": "check", "desc": "exists", "run": "test -f shared.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make base",
             "files_touched": ["shared.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "append",
             "files_touched": ["shared.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "fix c1".to_string(),
        findings: vec!["c1 wrong".to_string()],
        disposition: providers::VerifyDisposition::FixChunks {
            chunk_ids: vec!["c1".to_string()],
        },
    });

    let report = run_pipeline(&cfg, &ScriptedSpec::new(plan), &SharedFileChunks, &verify)
        .expect("a replay conflict yields a terminal report, not a hard error");
    assert_eq!(report.status, "rollback_conflict", "{report:#?}");
    assert!(!report.merged);
    // The failure reason names the kept chunk that could not replay (c2, the chunk
    // kept while its unseen dependency c1 was dropped).
    let failure = report.failure.as_deref().unwrap_or("");
    assert!(
        failure.contains("`c2`"),
        "failure should name the conflicting chunk c2: {failure:?}"
    );
    // The integration branch was restored intact (both chunks' work is still on it),
    // NOT left reset to the fork — proving the rebuild rolled back its own damage.
    assert!(super::git::branch_exists(repo.path(), "feat/demo"));
    assert_eq!(
        git_out(repo.path(), &["show", "feat/demo:shared.txt"]),
        "base\nextra",
        "feat must hold the intact pre-rollback content, not be reset to fork"
    );
}

#[test]
fn respec_rollback_conflict_leaves_run_state_consistent_with_the_old_branch() {
    // Item B transactional-audit fix (re-spec path): a re-spec whose provenance
    // rollback conflicts must NOT have already repointed the run to the new plan —
    // otherwise `finalize` emits a report for the new plan while the restored branch
    // still holds the OLD chunks. v1 = c1 (shared="base") + c2 (shared="base\nextra",
    // modifies shared, NO dep on c1). A SpecFlaw drops c1 to v2 (= c2 only); the
    // rollback keeps c2 and replays it onto the fork, which conflicts (c2's change
    // needs c1's shared.txt). The terminal report must describe the OLD plan: both
    // chunks still reported, plan_rev 1, respec_count 0, branch intact.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["shared.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 1,
        max_promotions: 0,
    };
    let v1 = json!({
        "acceptance": [{"kind": "check", "desc": "exists", "run": "test -f shared.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make base",
             "files_touched": ["shared.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "append",
             "files_touched": ["shared.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    let v2 = json!({
        "acceptance": [{"kind": "check", "desc": "exists", "run": "test -f shared.txt"}],
        "chunks": [
            {"id": "c2", "title": "b", "tier": "code", "brief": "append",
             "files_touched": ["shared.txt"], "checks": [{"desc": "b", "run": "true"}]},
        ],
    });
    let spec = ScriptedSpec::sequence(vec![v1, v2]);
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "c1 is a spec flaw; drop it".to_string(),
        findings: vec!["remove c1".to_string()],
        disposition: providers::VerifyDisposition::SpecFlaw {
            reason: "c1 is a spec flaw; drop it".to_string(),
            chunk_ids: vec!["c1".to_string()],
        },
    });

    let report = run_pipeline(&cfg, &spec, &SharedFileChunks, &verify)
        .expect("terminal report, not a crash");
    assert_eq!(report.status, "rollback_conflict", "{report:#?}");
    assert!(!report.merged);
    // The re-spec did NOT take effect: state still describes plan v1.
    assert_eq!(
        report.plan_rev, 1,
        "old plan revision, re-spec never landed"
    );
    assert_eq!(
        report.respec_count, 0,
        "respec_count must not advance when the rollback aborted the re-spec"
    );
    // Both chunks are still reported — the branch still holds them, so the audit must
    // not have pruned c1's report to the new-plan kept set.
    assert_eq!(
        report.chunks.len(),
        2,
        "both old-plan chunks still reported"
    );
    assert!(report.chunks.iter().any(|c| c.id == "c1"));
    assert!(report.chunks.iter().any(|c| c.id == "c2"));
    // The branch was restored intact with both chunks' content.
    assert!(super::git::branch_exists(repo.path(), "feat/demo"));
    assert_eq!(
        git_out(repo.path(), &["show", "feat/demo:shared.txt"]),
        "base\nextra"
    );
}

/// A per-chunk harness that writes `<id>.txt` keyed on the chunk id, so chunks
/// touch disjoint files and replay cleanly onto a rebuilt fork. Every call writes
/// the same content, so a re-run of a reverted chunk still produces a non-empty
/// diff off the fork.
struct DisjointChunks;
impl CodeHarness for DisjointChunks {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        let file = format!("{}.txt", req.chunk_id);
        std::fs::write(wt.join(&file), format!("{} content\n", req.chunk_id)).unwrap();
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        Ok(ChunkResult::committed(
            git_out(wt, &["rev-parse", "HEAD"]),
            vec![PathBuf::from(file)],
        ))
    }
}

#[test]
fn replayed_chunk_report_keeps_authored_commit_and_flags_replayed() {
    // Item E (replayed-chunk provenance fidelity): after a verify-FIX rollback
    // replays a KEPT chunk onto the rebuilt integration branch, its report must keep
    // the AUTHORED `commit` oid (the floor-gated tip) and record the replayed
    // on-branch oid in `merge_commit` + set `replayed = true` — NOT overwrite
    // `commit`. c1←c2 are stacked and both kept; the FIX targets only the
    // independent c3, so c1 and c2 are replayed while c3 is re-coded fresh. c2 is
    // replayed onto c1's REPLAYED tip (a different parent than its authored one), so
    // its replayed oid is guaranteed distinct from the authored `commit`.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["c1.txt", "c2.txt", "c3.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let plan = json!({
        "acceptance": [{"kind": "check", "desc": "all", "run": "test -f c1.txt && test -f c2.txt && test -f c3.txt"}],
        "chunks": [
            {"id": "c1", "title": "a", "tier": "code", "brief": "make c1",
             "files_touched": ["c1.txt"], "checks": [{"desc": "a", "run": "true"}]},
            {"id": "c2", "title": "b", "tier": "code", "brief": "make c2", "deps": ["c1"],
             "files_touched": ["c2.txt"], "checks": [{"desc": "b", "run": "true"}]},
            {"id": "c3", "title": "c", "tier": "code", "brief": "make c3",
             "files_touched": ["c3.txt"], "checks": [{"desc": "c", "run": "true"}]},
        ],
    });
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "fix c3".to_string(),
            findings: vec!["c3 wrong".to_string()],
            disposition: providers::VerifyDisposition::FixChunks {
                chunk_ids: vec!["c3".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "ok".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

    let report = run_pipeline(&cfg, &ScriptedSpec::new(plan), &DisjointChunks, &verify)
        .expect("pipeline runs");
    assert_eq!(report.status, "merged", "{report:#?}");

    let c1 = report.chunks.iter().find(|c| c.id == "c1").unwrap();
    let c2 = report.chunks.iter().find(|c| c.id == "c2").unwrap();
    let c3 = report.chunks.iter().find(|c| c.id == "c3").unwrap();
    // The two kept chunks were replayed by the rollback.
    assert!(c1.replayed, "c1 was kept through the rollback → replayed");
    assert!(c2.replayed, "c2 was kept through the rollback → replayed");
    // The re-code target was re-coded fresh (a normal no-ff merge), not replayed.
    assert!(!c3.replayed, "c3 was re-coded, not replayed");
    // Item E core: the authored `commit` is preserved distinct from the replayed
    // on-branch `merge_commit` (which now has a different parent than the authored
    // tip), instead of the old behaviour of overwriting `commit` with the replay.
    assert!(c2.commit.is_some() && c2.merge_commit.is_some());
    assert_ne!(
        c2.commit, c2.merge_commit,
        "authored commit must be preserved distinct from the replayed on-branch commit"
    );
    // The authored oid is still a real, resolvable commit object (not clobbered).
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["cat-file", "-e", c2.commit.as_deref().unwrap()])
            .status()
            .unwrap()
            .success(),
        "authored commit oid must remain a valid object"
    );
}

#[test]
fn nonlinear_chunk_history_is_rejected_at_gate() {
    // Item F (merge-commit inside a chunk range): a chunk whose history contains a
    // merge commit cannot be replayed by the provenance rollback (`git cherry-pick
    // base..commit` refuses a merge without `-m`), so it must be rejected at gate
    // time as a re-codable failure rather than becoming an un-replayable merged
    // chunk. With no re-code budget the run terminates `chunk_failed`, naming the
    // merge-commit reason.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));

    // A harness that produces a NON-LINEAR chunk history: it commits on the chunk
    // branch, then creates a side branch from the base, commits there, and merges it
    // back with --no-ff — leaving a merge commit inside `base..HEAD`.
    struct MergingHarness;
    impl CodeHarness for MergingHarness {
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
            _c: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            let wt = &req.worktree_path;
            let base = git_out(wt, &["rev-parse", "HEAD"]);
            // Main line commit on the chunk branch.
            std::fs::write(wt.join("feature.txt"), "main line\n").unwrap();
            git(wt, &["add", "-A"]);
            git(wt, &["commit", "-qm", "mainline"]);
            // A divergent side branch off the base, then merge it back (--no-ff) so a
            // real merge commit lands inside base..HEAD.
            git(wt, &["checkout", "-q", "-b", "side", &base]);
            std::fs::write(wt.join("other.txt"), "side\n").unwrap();
            git(wt, &["add", "-A"]);
            git(wt, &["commit", "-qm", "side"]);
            git(wt, &["checkout", "-q", "-"]);
            git(
                wt,
                &["merge", "--no-ff", "--no-edit", "-m", "merge side", "side"],
            );
            Ok(ChunkResult::committed(
                git_out(wt, &["rev-parse", "HEAD"]),
                vec![PathBuf::from("feature.txt"), PathBuf::from("other.txt")],
            ))
        }
    }

    let report = run_pipeline(&cfg, &spec, &MergingHarness, &ScriptedVerify::passing())
        .expect("pipeline runs");
    assert_eq!(report.status, "chunk_failed", "{report:#?}");
    assert!(!report.merged);
    let reason = report.chunks[0].reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("merge commit"),
        "block reason should name the non-linear merge-commit history: {reason:?}"
    );
    // Nothing reached source.
    assert!(!main_has(&repo, "feature.txt"));
}

#[test]
fn verify_fix_rollback_carries_reverted_chunks_prior_diff() {
    // Item L (carry the prior diff into verify-FIX re-codes): when a verify-FIX rolls
    // a merged chunk back, its reverted attempt's diff must seed the re-run's
    // re-brief — the code-stage `prior_diff` only covers in-visit floor retries, so
    // without this the first re-run after a rollback would forget the code it just
    // discarded. The re-code target's re-run brief must carry the fenced prior-diff
    // section with the reverted content.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    // Verify fails once (FIX c1) to force a rollback + re-code, then passes.
    let verify = ScriptedVerify::sequence(vec![
        providers::VerifyJudgment {
            passed: false,
            summary: "fix c1".to_string(),
            findings: vec!["needs work".to_string()],
            disposition: providers::VerifyDisposition::FixChunks {
                chunk_ids: vec!["c1".to_string()],
            },
        },
        providers::VerifyJudgment {
            passed: true,
            summary: "ok".to_string(),
            findings: vec![],
            disposition: providers::VerifyDisposition::Fix,
        },
    ]);

    // Records each attempt's brief; every call writes a unique content so the
    // reverted attempt has a real, identifiable diff.
    struct BriefRecorder {
        briefs: std::sync::Mutex<Vec<String>>,
        n: AtomicU32,
    }
    impl CodeHarness for BriefRecorder {
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
            _c: &CancelToken,
        ) -> Result<ChunkResult, HarnessError> {
            self.briefs.lock().unwrap().push(req.brief.clone());
            let n = self.n.fetch_add(1, Ordering::SeqCst);
            let wt = &req.worktree_path;
            let marker = if n == 0 {
                "REVERTED_ATTEMPT_MARKER"
            } else {
                "RECODED_MARKER"
            };
            std::fs::write(wt.join("feature.txt"), format!("{marker}\n")).unwrap();
            git(wt, &["add", "-A"]);
            git(wt, &["commit", "-qm", "edit"]);
            Ok(ChunkResult::committed(
                git_out(wt, &["rev-parse", "HEAD"]),
                vec![PathBuf::from("feature.txt")],
            ))
        }
    }

    let harness = BriefRecorder {
        briefs: std::sync::Mutex::new(Vec::new()),
        n: AtomicU32::new(0),
    };
    let report = run_pipeline(&cfg, &spec, &harness, &verify).expect("pipeline runs");
    assert_eq!(report.status, "merged", "{report:#?}");

    let briefs = harness.briefs.into_inner().unwrap();
    assert_eq!(
        briefs.len(),
        2,
        "one initial attempt + one post-rollback re-code"
    );
    // The initial attempt gets the bare brief — no prior-diff section.
    assert!(
        !briefs[0].contains("previous attempt's diff"),
        "initial brief must not carry a prior diff: {}",
        briefs[0]
    );
    // The post-rollback re-code carries the reverted attempt's diff as fenced DATA.
    assert!(
        briefs[1].contains("previous attempt's diff"),
        "post-rollback re-brief missing the carried prior diff: {}",
        briefs[1]
    );
    assert!(
        briefs[1].contains("REVERTED_ATTEMPT_MARKER"),
        "the carried diff must contain the reverted attempt's content: {}",
        briefs[1]
    );
}

/// Whether `path` exists on `main` (a committed blob).
fn main_has(repo: &TempDir, path: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["cat-file", "-e", &format!("main:{path}")])
        .status()
        .unwrap()
        .success()
}

// --- tiered triage: fast-coordinator seam + adaptive tier promotion (T6) ----

/// A two-rung [`TierHarness`]: the base tier runs `base`, every higher tier runs
/// `promoted` (and flips `promoted_ran`). Models the live ladder deterministically
/// so a promotion actually swaps which "agent" codes the chunk.
struct TwoTier<'a> {
    base: &'a dyn CodeHarness,
    promoted: &'a dyn CodeHarness,
    promoted_ran: &'a std::sync::atomic::AtomicBool,
}
impl TierHarness for TwoTier<'_> {
    fn harness(&self, tier: Tier) -> &dyn CodeHarness {
        if tier == Tier::Code {
            self.base
        } else {
            self.promoted_ran.store(true, Ordering::SeqCst);
            self.promoted
        }
    }
}

/// A [`Decider`] spy: records the [`Action::name`] of every consequential proposal
/// it is asked to rule on, and confirms it unchanged. Lets a test assert that
/// **routine** decisions never reach the expensive tier.
struct SpyDecider {
    seen: std::cell::RefCell<Vec<String>>,
}
impl SpyDecider {
    fn new() -> Self {
        Self {
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }
}
impl Decider for SpyDecider {
    fn decide_consequential(
        &self,
        _ctx: &DecisionContext,
        proposed: &crate::pipeline::CoordinatorProposal,
    ) -> DeciderVerdict {
        self.seen
            .borrow_mut()
            .push(proposed.action.name().to_string());
        DeciderVerdict {
            action: proposed.action.clone(),
            reason: proposed.reason.clone(),
            input_artifacts: proposed.input_artifacts.clone(),
        }
    }
    fn model(&self) -> String {
        "spy-opus".to_string()
    }
    fn prompt_version(&self) -> String {
        "v1".to_string()
    }
}

/// A [`Decider`] that OVERRIDES **any** consequential proposal (`DECLARE_CONVERGED`
/// or `TRIGGER_RE_SPEC`) with `ESCALATE` — the seam the circuit-breaker layer forces
/// later (design §0.2/§2).
struct EscalatingDecider;
impl Decider for EscalatingDecider {
    fn decide_consequential(
        &self,
        _ctx: &DecisionContext,
        proposed: &crate::pipeline::CoordinatorProposal,
    ) -> DeciderVerdict {
        DeciderVerdict {
            action: Action::Escalate {
                reason: "decider withheld the consequential decision".to_string(),
            },
            reason: "not actually done".to_string(),
            input_artifacts: proposed.input_artifacts.clone(),
        }
    }
    fn model(&self) -> String {
        "escalating-opus".to_string()
    }
    fn prompt_version(&self) -> String {
        "v1".to_string()
    }
}

/// A [`TierHarness`] that runs ONE (failing) harness at every tier but keeps the
/// full `code → mid → high` promotion ladder — for exercising promotion all the way
/// to the ceiling, where the repeated-failure breaker finally trips.
struct FullLadder<'a>(&'a dyn CodeHarness);
impl TierHarness for FullLadder<'_> {
    fn harness(&self, _tier: Tier) -> &dyn CodeHarness {
        self.0
    }
}

#[test]
fn repeat_fail_promotes_chunk_to_a_higher_tier() {
    // Design §3 adaptive promotion: a chunk whose per-tier re-code budget is spent
    // is re-run at the NEXT model tier up instead of tripping the breaker. The base
    // tier here can never pass (it always writes an out-of-scope stray), so a merge
    // is only reachable by promoting to a harness that stays in scope.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 1,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));

    let base = AlwaysStray; // persistent floor failure at the base tier
    let promoted = CommitFake::new(&[("feature.txt", "hi\n")]); // clean at the higher tier
    let promoted_ran = std::sync::atomic::AtomicBool::new(false);
    let harnesses = TwoTier {
        base: &base,
        promoted: &promoted,
        promoted_ran: &promoted_ran,
    };
    let decider = crate::pipeline::ScriptedDecider::confirming();

    let report = run_pipeline_tiered(
        &cfg,
        &spec,
        &harnesses,
        &ScriptedVerify::passing(),
        &decider,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    assert_eq!(report.promote_count, 1, "exactly one PROMOTE_TIER happened");
    assert!(
        promoted_ran.load(Ordering::SeqCst),
        "the promoted (higher-tier) harness must have run"
    );
    // The report reflects the promoted tier the chunk finally merged at.
    assert_eq!(report.chunks[0].tier, "mid");
    // A PROMOTE_TIER decision was recorded (routine → coordinator-tier).
    let promote = report
        .decisions
        .iter()
        .find(|d| d.reason.contains("promote chunk"))
        .expect("a promote decision is recorded");
    assert_eq!(promote.decision_tier, DecisionTier::Coordinator);
}

#[test]
fn promotion_is_bounded_by_max_promotions_then_the_breaker_trips() {
    // With promotion budget 0 the pre-promotion behaviour is preserved: a
    // persistently-failing chunk trips the repeated-failure breaker, never promotes.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    let promoted = CommitFake::new(&[("feature.txt", "hi\n")]);
    let promoted_ran = std::sync::atomic::AtomicBool::new(false);
    let base = AlwaysStray;
    let harnesses = TwoTier {
        base: &base,
        promoted: &promoted,
        promoted_ran: &promoted_ran,
    };
    let decider = crate::pipeline::ScriptedDecider::confirming();

    let report = run_pipeline_tiered(
        &cfg,
        &spec,
        &harnesses,
        &ScriptedVerify::passing(),
        &decider,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert_eq!(report.promote_count, 0, "no promotion with budget 0");
    assert!(
        !promoted_ran.load(Ordering::SeqCst),
        "the higher tier must never run when promotion is disabled"
    );
    assert!(!report.merged);
}

#[test]
fn routine_decisions_never_reach_the_decider_only_converge_does() {
    // Done criterion: a routine decision (RE_CODE_CHUNK, PROMOTE_TIER) is emitted
    // by the fast coordinator directly and does NOT hit the Opus decider; only a
    // consequential decision (DECLARE_CONVERGED) is deferred to it. The promotion
    // scenario exercises a RE_CODE (routine) AND a PROMOTE_TIER (routine) before it
    // converges — the spy decider must see exactly one call, for declare_converged.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 1,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 1,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    let base = AlwaysStray;
    let promoted = CommitFake::new(&[("feature.txt", "hi\n")]);
    let promoted_ran = std::sync::atomic::AtomicBool::new(false);
    let harnesses = TwoTier {
        base: &base,
        promoted: &promoted,
        promoted_ran: &promoted_ran,
    };
    let decider = SpyDecider::new();

    let report = run_pipeline_tiered(
        &cfg,
        &spec,
        &harnesses,
        &ScriptedVerify::passing(),
        &decider,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "merged", "{report:#?}");
    // A routine RE_CODE and a routine PROMOTE both happened...
    assert_eq!(report.recode_count, 1);
    assert_eq!(report.promote_count, 1);
    // ...yet the decider was consulted exactly once, and only for the ship decision.
    let seen = decider.seen.into_inner();
    assert_eq!(seen, vec!["declare_converged".to_string()], "{seen:?}");
}

#[test]
fn decider_may_override_converge_with_escalate() {
    // The consequential-decision seam is real both ways: a decider that overrides
    // DECLARE_CONVERGED with ESCALATE stops the merge (design §0.2/§2) — the exact
    // hook the circuit-breaker layer forces later.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let harnesses = SingleTierHarness(&code);

    let report = run_pipeline_tiered(
        &cfg,
        &spec,
        &harnesses,
        &ScriptedVerify::passing(),
        &EscalatingDecider,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "escalated", "{report:#?}");
    assert!(!report.merged, "an escalated feature must not land");
    assert!(!main_has(&repo, "feature.txt"));
    // The recorded ship decision is the decider's ESCALATE, decider-tier.
    let escalate = report
        .decisions
        .iter()
        .find(|d| d.reason.contains("not actually done"))
        .expect("the decider's escalate verdict is recorded");
    assert_eq!(escalate.decision_tier, DecisionTier::Decider);
    // The chunk work is NOT lost: the integration branch (with the merged chunk) is
    // preserved for recovery (state-integrity invariant 5), since it holds commits
    // ahead of source that never reached it.
    assert!(
        super::git::branch_exists(repo.path(), &report.integration_branch),
        "escalated work must remain on the preserved integration branch"
    );
}

#[test]
fn promotion_climbs_the_whole_ladder_then_the_breaker_trips() {
    // Design §3 + §9: a chunk that keeps failing is promoted up the FULL ladder
    // (code → mid → high) and, once the ceiling is reached with no higher tier to
    // try, the repeated-failure breaker finally trips. Here every tier runs the
    // same persistently-failing harness, so no promotion can rescue it.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0, // straight to promotion on each failure
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 5, // more than the ladder height; the ladder is the real bound
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    let failing = AlwaysStray;
    let harnesses = FullLadder(&failing);
    let decider = crate::pipeline::ScriptedDecider::confirming();

    let report = run_pipeline_tiered(
        &cfg,
        &spec,
        &harnesses,
        &ScriptedVerify::passing(),
        &decider,
    )
    .expect("pipeline runs");

    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    // code→mid and mid→high: exactly two promotions, then the ladder is exhausted.
    assert_eq!(
        report.promote_count, 2,
        "promoted to the ceiling, no further"
    );
    assert!(!report.merged);
}

#[test]
fn decider_may_override_respec_with_escalate() {
    // The consequential seam guards TRIGGER_RE_SPEC too: a SPEC-FLAW verdict would
    // normally re-plan, but a decider that escalates it stops the loop — no new plan
    // revision is produced.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 0,
        max_fix_iterations: 2,
        max_respec: 1,
        max_promotions: 0,
    };
    let spec = ScriptedSpec::new(one_chunk_plan(
        &["feature.txt"],
        "true",
        "test -f feature.txt",
    ));
    let verify = ScriptedVerify::new(providers::VerifyJudgment {
        passed: false,
        summary: "the plan cannot meet intent".to_string(),
        findings: vec!["wrong approach".to_string()],
        disposition: providers::VerifyDisposition::SpecFlaw {
            reason: "the plan cannot meet intent".to_string(),
            chunk_ids: vec!["c1".to_string()],
        },
    });
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let harnesses = SingleTierHarness(&code);

    let report = run_pipeline_tiered(&cfg, &spec, &harnesses, &verify, &EscalatingDecider)
        .expect("pipeline runs");

    assert_eq!(report.status, "escalated", "{report:#?}");
    assert!(!report.merged);
    assert_eq!(
        report.respec_count, 0,
        "an escalated re-spec is not counted"
    );
    assert_eq!(report.plan_rev, 1, "no new plan revision was produced");
    // The spec provider was never asked to re-plan.
    assert!(spec.respec_calls().is_empty());
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
        "schema_version": 3, "plan_rev": 1, "intent_rev": 1,
        "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
        "baseline": {"ref": "feat/f@fork", "commit_oid": "0123456789abcdef0123456789abcdef01234567", "toolchain": "rustc 1.97.1", "test_passlist_hash": "h", "clippy_warnings_hash": "h", "enumerated_targets_hash": "h"},
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
    // A live run may or may not merge (depends on the real agents), but the loop
    // must reach a well-formed terminal state with a real plan.
    assert!(
        report.chunk_count >= 1,
        "spec must produce at least one chunk"
    );
    assert!(
        [
            "merged",
            "chunk_floor_blocked",
            "chunk_failed",
            "chunk_merge_conflict",
            "verify_failed",
            "floor_blocked",
            "escalated",
            "merge_conflict",
        ]
        .contains(&report.status.as_str()),
        "unexpected terminal status: {}",
        report.status
    );
    if report.status == "merged" {
        assert!(report.merged && report.final_commit.is_some());
    }
}

// --- Resource circuit-breakers (design §9) ----------------------------------

/// A [`CodeHarness`] that commits `files` in the chunk worktree AND reports a
/// fixed [`Usage`], so the cost/token breakers have real spend to meter.
struct MeteredFake {
    files: Vec<(&'static str, &'static str)>,
    usage: Usage,
}
impl CodeHarness for MeteredFake {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: true,
            honors_file_scope: false,
            runs_checks: false,
        }
    }
    fn run_chunk(&self, req: &ChunkRequest, _c: &CancelToken) -> Result<ChunkResult, HarnessError> {
        let wt = &req.worktree_path;
        let mut changed = Vec::new();
        for (rel, content) in &self.files {
            std::fs::write(wt.join(rel), content).unwrap();
            changed.push(PathBuf::from(*rel));
        }
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", "edit"]);
        let mut res = ChunkResult::committed(git_out(wt, &["rev-parse", "HEAD"]), changed);
        res.usage = Some(self.usage.clone());
        Ok(res)
    }
}

fn usage(tokens: u64, cost: f64) -> Usage {
    Usage {
        input_tokens: None,
        output_tokens: None,
        total_tokens: Some(tokens),
        cost_usd: Some(cost),
    }
}

#[test]
fn cost_ceiling_breaker_aborts_regardless_of_convergence() {
    // Design §9 cost ceiling + kill-switch: a chunk whose metered spend exceeds
    // the ceiling aborts the run BEFORE it can merge the feature, even though the
    // chunk itself floor-passed and verify would have converged.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_cost_usd: Some(1.0),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = MeteredFake {
        files: vec![("feature.txt", "hi\n")],
        usage: usage(15, 5.0), // $5 spend > $1 ceiling
    };
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("cost ceiling"),
        "{report:#?}"
    );
    assert!(
        !report.merged,
        "the cost breaker aborts before the feature merges"
    );
    assert!(
        report.resources.cost_usd >= 5.0,
        "spend was metered from Usage"
    );
}

#[test]
fn token_ceiling_breaker_aborts() {
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_total_tokens: Some(1_000),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = MeteredFake {
        files: vec![("feature.txt", "hi\n")],
        usage: usage(5_000, 0.0), // 5000 tokens > 1000 ceiling
    };
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("token ceiling"),
        "{report:#?}"
    );
    assert!(report.resources.total_tokens >= 5_000);
}

#[test]
fn wall_time_ceiling_breaker_aborts() {
    // A 1ns ceiling is crossed by the time the loop is entered (spec already ran),
    // so the wall-time breaker trips at the round boundary before any code stage.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_wall_time: Some(std::time::Duration::from_nanos(1)),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("wall-time"),
        "{report:#?}"
    );
    assert!(!report.merged);
}

#[test]
fn process_count_ceiling_breaker_aborts() {
    // Ceiling 1: the spec invocation alone reaches it; the first chunk attempt
    // crosses it, aborting before the feature merges.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_processes: Some(1),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("process-count"),
        "{report:#?}"
    );
    assert!(report.resources.processes >= 2, "spec + chunk were counted");
}

#[test]
fn storage_ceiling_breaker_aborts() {
    // A 1-byte ceiling is crossed by the scratch workdir (which already holds the
    // integration worktree), so the storage breaker trips at the round boundary.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_storage_bytes: Some(1),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("storage ceiling"),
        "{report:#?}"
    );
    assert!(
        report.resources.storage_bytes > 1,
        "workdir size was measured"
    );
}

#[test]
fn repeated_identical_failure_breaker_aborts_before_exhausting_recode_budget() {
    // Design §9 repeated-identical-failure: the SAME floor block recurring twice
    // aborts even though the re-code budget (5) would otherwise keep grinding — the
    // breaker is a tighter bound than the raw attempt count.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.fix_loop = FixLoopConfig {
        max_recode_per_chunk: 5,
        max_fix_iterations: 0,
        max_respec: 0,
        max_promotions: 0,
    };
    cfg.budget = ResourceBudget {
        max_identical_failures: Some(2),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let report = run_pipeline(&cfg, &spec, &AlwaysStray, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("repeated-identical-failure"),
        "{report:#?}"
    );
    // Exactly one re-code happened before the breaker fired at the 2nd identical
    // block — far short of the budget of 5.
    assert_eq!(report.recode_count, 1, "{report:#?}");
    assert!(!report.merged);
}

#[test]
fn unlimited_budget_does_not_disturb_a_clean_merge() {
    // The resource breakers are additive: with UNLIMITED (the test default) a
    // normal run still merges, and the tally is surfaced on the report.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = MeteredFake {
        files: vec![("feature.txt", "hi\n")],
        usage: usage(42, 0.01),
    };
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "merged", "{report:#?}");
    assert!(report.circuit_breaker.is_none());
    assert_eq!(
        report.resources.total_tokens, 42,
        "tally surfaced on the report"
    );
    assert!(
        report.resources.processes >= 3,
        "spec + chunk + verify counted"
    );
}

#[test]
fn resolve_ceilings_map_zero_to_disabled_and_reject_nonfinite() {
    // u64/u32: Some(0) disables, other values override, None → default.
    assert_eq!(super::resolve_u64_ceiling(Some(0), Some(99)), None);
    assert_eq!(super::resolve_u64_ceiling(Some(5), Some(99)), Some(5));
    assert_eq!(super::resolve_u64_ceiling(None, Some(99)), Some(99));
    assert_eq!(super::resolve_u32_ceiling(Some(0), Some(3)), None);
    assert_eq!(super::resolve_u32_ceiling(None, Some(3)), Some(3));

    // f64: only a finite positive enables; NaN/Inf/0/negative all disable.
    assert_eq!(
        super::resolve_f64_ceiling(Some(10.0), Some(1.0)),
        Some(10.0)
    );
    assert_eq!(super::resolve_f64_ceiling(Some(0.0), Some(1.0)), None);
    assert_eq!(super::resolve_f64_ceiling(Some(-5.0), Some(1.0)), None);
    assert_eq!(super::resolve_f64_ceiling(Some(f64::NAN), Some(1.0)), None);
    assert_eq!(
        super::resolve_f64_ceiling(Some(f64::INFINITY), Some(1.0)),
        None
    );
    assert_eq!(super::resolve_f64_ceiling(None, Some(1.0)), Some(1.0));
}

#[test]
fn verify_stage_breach_aborts_instead_of_merging() {
    // Finding #8 from review: a verify invocation that crosses a ceiling must abort
    // BEFORE the run merges. With max_processes = 2: spec(1) + one chunk(2) pass the
    // post-merge check (2 is not > 2), but the verify call makes it 3 > 2, so the
    // verify-stage breaker check fires and the feature does not merge.
    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.budget = ResourceBudget {
        max_processes: Some(2),
        ..ResourceBudget::UNLIMITED
    };
    let spec = ScriptedSpec::new(one_chunk_plan(&["feature.txt"], "true", "true"));
    let code = CommitFake::new(&[("feature.txt", "hi\n")]);
    let report = run_pipeline(&cfg, &spec, &code, &ScriptedVerify::passing()).expect("runs");
    assert_eq!(report.status, "circuit_breaker", "{report:#?}");
    assert!(
        report
            .circuit_breaker
            .as_deref()
            .unwrap_or_default()
            .contains("process-count"),
        "{report:#?}"
    );
    assert!(!report.merged, "verify crossed the ceiling → no merge");
    assert!(
        report.resources.processes >= 3,
        "spec + chunk + verify counted"
    );
}

#[test]
fn capture_snapshot_allocates_a_fresh_target_dir_per_call() {
    // done-criteria (c), end to end: every `capture_snapshot` call must get its
    // OWN `CARGO_TARGET_DIR` so baseline and tip never share a warm clippy cache
    // (a shared cache re-emits zero warnings → no-new-clippy passes vacuously).
    // A fake test/clippy command appends the CARGO_TARGET_DIR it saw to a log;
    // two capture_snapshot calls must record two different dirs.
    use std::os::unix::fs::PermissionsExt;

    let repo = init_repo();
    let workdir = TempDir::new().unwrap();
    let log = workdir.path().join("seen-target-dirs.log");
    let script = workdir.path().join("record.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$CARGO_TARGET_DIR\" >> '{}'\nprintf '{{\"reason\":\"build-finished\",\"success\":true}}\\n'\n",
            log.display()
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();

    let mut cfg = config(repo.path(), workdir.path(), &["feature.txt"]);
    cfg.test_cmd = script.to_string_lossy().into_owned();
    cfg.clippy_cmd = script.to_string_lossy().into_owned();

    super::capture_snapshot(&cfg, repo.path()).unwrap();
    super::capture_snapshot(&cfg, repo.path()).unwrap();

    let recorded = std::fs::read_to_string(&log).unwrap();
    let dirs: Vec<&str> = recorded.lines().collect();
    // Two captures × (test + clippy) = 4 lines.
    assert_eq!(dirs.len(), 4, "each capture runs test + clippy: {recorded}");
    // All four dirs are distinct: test and clippy get separate dirs within a
    // snapshot (clippy can't reuse the test build's warm artifacts), and the two
    // capture_snapshot calls never share (no cross-ref cache sharing).
    let unique: std::collections::BTreeSet<&str> = dirs.iter().copied().collect();
    assert_eq!(
        unique.len(),
        4,
        "every capture must get its own target dir: {recorded}"
    );
    // And each is non-empty (the floor really pinned one).
    assert!(dirs.iter().all(|d| !d.is_empty()));
}
