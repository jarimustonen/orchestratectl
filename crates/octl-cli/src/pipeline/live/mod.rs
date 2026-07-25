//! The live, end-to-end code-pipeline driver (design.md §6 call diagram — the
//! FIRST bold-to-live wiring, breakdown T5 walking skeleton).
//!
//! This is the additive `orchestratectl pipeline run` command: it runs one
//! single feature through the whole loop —
//! **spec[Opus] → code[claude-deepseek] → floor-gate → verify[Opus] → merge** —
//! reusing every landed piece behind the seam:
//!
//! - the [`CodeHarness`](crate::harness::CodeHarness) adapters (the code stage
//!   is [`ClaudeHarness::deepseek`](crate::harness::claude::ClaudeHarness::deepseek));
//! - the deterministic [`floor`](crate::floor) as the hard merge gate (design §4);
//! - the `plan.json` v2 types + validator ([`octl_core::plan`], design §13);
//! - the [`DecisionEnvelope`](crate::pipeline::DecisionEnvelope) audit record
//!   (design §2), stamped with the tier that made each call.
//!
//! # Additive, not a `run create`
//!
//! This command does **not** create an orchestratectl run, append events, or
//! touch the supervisor / reducer / lock layer — it keeps its own scratch state
//! (intent.md, plan.json, transcripts, git worktrees) under a work dir and emits
//! a structured [`PipelineReport`]. That keeps it clear of the five
//! state-integrity invariants (no new raw event-append path) while it proves the
//! whole system end to end.
//!
//! # Scope (v1 skeleton)
//!
//! Happy path only. The verify→triage→fix loop, re-spec, tier promotion, and the
//! tiered fast-coordinator triage are deferred to follow-ups (filed as issues);
//! a chunk or feature that fails the floor is preserved and the run stops — it is
//! never merged (the floor is the hard gate, design §4/§14).

pub mod git;
pub mod providers;

use std::path::{Path, PathBuf};
use std::time::Duration;

use octl_core::plan::{self, Acceptance, Chunk, Plan};
use serde::Serialize;

use crate::error::CliError;
use crate::floor::{
    self, evaluate_floor, BaselineSnapshot, CheckRun, FloorInputs, FloorVerdict, RunSnapshot,
};
use crate::harness::{CancelToken, Check as HarnessCheck, ChunkOutcome, ChunkRequest, CodeHarness};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::pipeline::{DecisionEnvelope, DecisionTier};

use git::MergeOutcome;
use providers::{SpecContext, SpecProvider, VerifyContext, VerifyJudgment, VerifyProvider};

/// A failure in the live pipeline. Mapped to a [`CliError`] at the command
/// boundary; each variant carries a stable code for the error envelope.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// A git shell-out failed.
    #[error("git error: {0}")]
    Git(String),
    /// Bad input / setup precondition (repo, branch, workdir).
    #[error("setup error: {0}")]
    Setup(String),
    /// The spec stage could not be driven to a candidate plan.
    #[error("spec stage failed: {0}")]
    Spec(String),
    /// The spec produced a plan that failed the T2 validator (even after retry).
    #[error("plan invalid: {0}")]
    PlanInvalid(String),
    /// The verify stage could not be driven to a verdict.
    #[error("verify stage failed: {0}")]
    Verify(String),
    /// The floor's capture layer could not collect what it needs to judge.
    #[error("floor capture error: {0}")]
    Floor(String),
    /// The code harness could not produce a result at all.
    #[error("harness error: {0}")]
    Harness(String),
    /// An I/O failure writing scratch state.
    #[error("io error: {0}")]
    Io(String),
}

impl PipelineError {
    /// A stage-scoped error (`spec`/`verify`) carrying the underlying message.
    fn stage(stage: &str, message: impl Into<String>) -> Self {
        match stage {
            "spec" => PipelineError::Spec(message.into()),
            _ => PipelineError::Verify(message.into()),
        }
    }

    /// Stable error code for the CLI error envelope.
    fn code(&self) -> &'static str {
        match self {
            PipelineError::Git(_) => "git_error",
            PipelineError::Setup(_) => "setup_error",
            PipelineError::Spec(_) => "spec_failed",
            PipelineError::PlanInvalid(_) => "plan_invalid",
            PipelineError::Verify(_) => "verify_failed",
            PipelineError::Floor(_) => "floor_error",
            PipelineError::Harness(_) => "harness_error",
            PipelineError::Io(_) => "io_error",
        }
    }
}

impl From<floor::FloorError> for PipelineError {
    fn from(e: floor::FloorError) -> Self {
        PipelineError::Floor(e.to_string())
    }
}

impl From<PipelineError> for CliError {
    fn from(e: PipelineError) -> Self {
        let code = e.code();
        // A bad plan or bad setup is the caller's problem (User); everything
        // else is a system/IO/tooling failure the caller cannot fix by
        // re-phrasing input.
        match e {
            PipelineError::PlanInvalid(_) | PipelineError::Setup(_) => {
                CliError::user(code, e.to_string())
            }
            _ => CliError::system(code, e.to_string()),
        }
    }
}

/// Fully-resolved configuration for one pipeline run.
pub struct PipelineConfig {
    /// Git repository to operate on (a path inside it; the toplevel is derived).
    pub repo: PathBuf,
    /// The intent text (already resolved from a string or a file).
    pub intent: String,
    /// Branch the feature forks from and (on success) merges back to.
    pub source_branch: String,
    /// Optional file-scope hint passed to the spec stage.
    pub files: Vec<PathBuf>,
    /// Optional slug override (else derived from the intent).
    pub slug: Option<String>,
    /// Shell command that captures the test pass-list for the floor baseline +
    /// current snapshots (default `cargo test`).
    pub test_cmd: String,
    /// Shell command that captures the clippy warning-list (default
    /// `cargo clippy --message-format=short`).
    pub clippy_cmd: String,
    /// Scratch root for worktrees + artifacts (intent.md, plan.json, transcripts).
    pub workdir: PathBuf,
    /// How many out-of-scope files the floor tolerates before failing file-scope.
    pub file_scope_slack: usize,
    /// Keep worktrees/branches after the run (skip teardown) for debugging.
    pub keep: bool,
    /// Optional per-chunk wall-clock ceiling for the code harness.
    pub chunk_timeout: Option<Duration>,
}

/// One chunk's outcome in the report.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkReport {
    /// Chunk id.
    pub id: String,
    /// Chunk title.
    pub title: String,
    /// Starting tier (wire name).
    pub tier: String,
    /// Harness outcome: `committed | no_change | failed | timeout | cancelled`.
    pub outcome: String,
    /// Whether the floor passed (`None` when the chunk produced no commit to gate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_passed: Option<bool>,
    /// The full floor verdict, when the floor ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<FloorVerdict>,
    /// Whether the chunk was merged into the integration branch.
    pub merged: bool,
    /// The chunk's own resulting commit (the harness-produced, floor-gated oid),
    /// when it committed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The integration-branch merge commit that folded the chunk in, when merged
    /// (distinct from `commit`, the chunk's own tip — so provenance is unambiguous).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_commit: Option<String>,
    /// A failure/blocked reason, when not merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The preserved branch name, when the chunk was kept for inspection
    /// (state-integrity invariant 5: unmerged work is preserved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_preserved: Option<String>,
}

/// The verify stage's result in the report.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    /// Whether every executable acceptance check passed (the mechanical half).
    pub acceptance_checks_passed: bool,
    /// Whether the LLM judged product-vs-intent a pass.
    pub judged_passed: bool,
    /// The combined verdict (`acceptance_checks_passed && judged_passed`).
    pub passed: bool,
    /// One-line judge summary.
    pub summary: String,
    /// Judge findings (recorded, not looped on in v1).
    pub findings: Vec<String>,
}

/// The structured summary the command emits (text + `--json`).
#[derive(Debug, Clone, Serialize)]
pub struct PipelineReport {
    /// Feature slug.
    pub slug: String,
    /// Source branch.
    pub source_branch: String,
    /// Integration branch.
    pub integration_branch: String,
    /// Intent revision (always 1 in the v1 skeleton — no re-spec).
    pub intent_rev: u32,
    /// Plan revision (always 1 in the v1 skeleton).
    pub plan_rev: u32,
    /// Number of chunks in the plan.
    pub chunk_count: usize,
    /// Per-chunk outcome + floor verdict.
    pub chunks: Vec<ChunkReport>,
    /// Verify result, when the pipeline reached the verify stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyReport>,
    /// The feature-level floor verdict at the tip, when the final re-check ran —
    /// so a `floor_blocked` status names exactly which gate failed, rather than
    /// hiding it in the verify summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_floor: Option<FloorVerdict>,
    /// Whether the feature merged into the source branch.
    pub merged: bool,
    /// The final commit on the source branch, when merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_commit: Option<String>,
    /// Overall status: `merged | floor_blocked | verify_failed | chunk_failed`.
    pub status: String,
    /// Decision envelopes recording the tier that made each call (design §2).
    pub decisions: Vec<DecisionEnvelope>,
    /// A terminal failure reason, when the pipeline could not complete the loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// Derive a filesystem-/branch-safe slug from the intent's first non-empty line.
/// Lowercased, non-alphanumerics collapsed to single hyphens, trimmed, capped —
/// and guaranteed non-empty (falls back to `feature`) so it satisfies the plan
/// validator's `feature.slug` and forms a valid `feat/<slug>` branch.
#[must_use]
pub fn slugify(intent: &str) -> String {
    let seed = intent.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut slug = String::new();
    let mut prev_hyphen = false;
    for ch in seed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen && !slug.is_empty() {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(48).collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "feature".to_string()
    } else {
        slug
    }
}

/// Resolve the `--intent` argument: if it names an existing file (or is prefixed
/// with `@`), read that file; otherwise treat it as the intent text verbatim.
///
/// # Errors
///
/// Returns [`PipelineError::Setup`] when a named file cannot be read, or when the
/// resolved intent is empty.
pub fn resolve_intent(raw: &str) -> Result<String, PipelineError> {
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path)
            .map_err(|e| PipelineError::Setup(format!("could not read intent file {path}: {e}")))?
    } else if Path::new(raw).is_file() {
        std::fs::read_to_string(raw)
            .map_err(|e| PipelineError::Setup(format!("could not read intent file {raw}: {e}")))?
    } else {
        raw.to_string()
    };
    if text.trim().is_empty() {
        return Err(PipelineError::Setup("intent is empty".to_string()));
    }
    Ok(text)
}

/// Order chunks so every chunk appears after all of its `deps` (a stable
/// topological sort). The plan validator guarantees the graph is acyclic and all
/// deps resolve, so this always succeeds; ties break on the plan's declared
/// order for determinism.
fn topo_order(chunks: &[Chunk]) -> Vec<usize> {
    use std::collections::HashMap;
    let index: HashMap<&str, usize> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let mut done = vec![false; chunks.len()];
    let mut order = Vec::with_capacity(chunks.len());
    // Repeatedly emit the first not-yet-done chunk whose deps are all done.
    while order.len() < chunks.len() {
        let mut progressed = false;
        for (i, c) in chunks.iter().enumerate() {
            if done[i] {
                continue;
            }
            let ready = c
                .deps
                .iter()
                .all(|d| index.get(d.as_str()).is_some_and(|&j| done[j]));
            if ready {
                done[i] = true;
                order.push(i);
                progressed = true;
            }
        }
        if !progressed {
            // Unreachable for a validated (acyclic) plan; emit the remainder in
            // declared order rather than loop forever.
            for (i, _) in chunks.iter().enumerate() {
                if !done[i] {
                    order.push(i);
                    done[i] = true;
                }
            }
        }
    }
    order
}

/// Convert a plan check into the harness `Check` the agent runs as its own
/// self-check (the authoritative floor check is run separately by
/// [`floor::runner`]). Synthesizes a stable id since `plan::Check` carries none.
fn to_harness_check(i: usize, c: &plan::Check) -> HarnessCheck {
    HarnessCheck {
        id: format!("chk-{i}"),
        desc: c.desc.clone(),
        run: c.run.clone(),
        timeout: None,
    }
}

/// Capture a [`RunSnapshot`] (tests + clippy) in `dir` using the configured
/// commands.
fn capture_snapshot(cfg: &PipelineConfig, dir: &Path) -> Result<RunSnapshot, PipelineError> {
    let tests = floor::runner::capture_test_snapshot(&cfg.test_cmd, dir)?;
    let clippy = floor::runner::capture_clippy_snapshot(&cfg.clippy_cmd, dir)?;
    Ok(RunSnapshot {
        tests,
        clippy,
        coverage: None,
    })
}

/// Build a decision envelope stamped with the deciding tier (design §2).
fn envelope(
    actor: &str,
    tier: DecisionTier,
    reason: impl Into<String>,
    inputs: Vec<String>,
    model: impl Into<String>,
    prompt_version: impl Into<String>,
) -> DecisionEnvelope {
    DecisionEnvelope {
        actor: actor.to_string(),
        input_artifacts: inputs,
        reason: reason.into(),
        decision_tier: tier,
        model: model.into(),
        prompt_version: prompt_version.into(),
    }
}

/// Internal running state threaded through the driver's stages so teardown and
/// the report can see what was created.
struct Run<'a> {
    cfg: &'a PipelineConfig,
    repo: PathBuf,
    slug: String,
    integration_branch: String,
    integration_wt: PathBuf,
    fork_commit: String,
    decisions: Vec<DecisionEnvelope>,
    chunk_reports: Vec<ChunkReport>,
    /// The feature-level floor verdict, once the final re-check runs.
    feature_floor: Option<FloorVerdict>,
    /// Set when the code stage stopped a chunk short; names the terminal status
    /// (`chunk_floor_blocked` vs `chunk_failed` vs `chunk_merge_conflict`).
    code_block_status: Option<&'static str>,
    /// Chunk (worktree, branch) pairs preserved because they were not merged.
    preserved: Vec<(PathBuf, String)>,
    merged_to_source: bool,
}

impl Drop for Run<'_> {
    /// Teardown runs unconditionally when the `Run` goes out of scope — on every
    /// success AND every error path — so no `?` early-return can leak a worktree
    /// or branch (the manual-teardown-per-return approach reviewers flagged as
    /// leaky). [`teardown`] is idempotent-safe and honours `--keep`.
    fn drop(&mut self) {
        teardown(self);
    }
}

/// Run the whole pipeline for one feature and return the structured report.
///
/// The stages ([`SpecProvider`], [`CodeHarness`], [`VerifyProvider`]) are
/// injected so the orchestration logic is unit-testable with deterministic
/// stubs; the live command wires the real Claude/deepseek implementations.
///
/// # Errors
///
/// Returns a [`PipelineError`] for a hard failure (bad repo/branch, an
/// undriveable stage, a git or floor-capture failure). A *floor block* or a
/// failed *verify* is NOT an error — it is a completed run whose
/// [`PipelineReport::status`] records the block; the report is still returned so
/// the caller can inspect the per-chunk floor verdicts.
pub fn run_pipeline(
    cfg: &PipelineConfig,
    spec: &dyn SpecProvider,
    code: &dyn CodeHarness,
    verify: &dyn VerifyProvider,
) -> Result<PipelineReport, PipelineError> {
    // --- 1. Setup: validate repo/branch, fork the integration branch, snapshot
    //        the baseline, write intent.md. Every fallible step after the branch
    //        is created runs under `run`, whose `Drop` guarantees teardown. ---
    let repo = git::toplevel(&cfg.repo)?;
    // The source MUST be a real local branch — a tag / remote-tracking ref /
    // `HEAD` would resolve but then `git worktree add` and the final merge would
    // target a detached or non-updatable ref and the run would "merge" nowhere.
    if !git::branch_exists(&repo, &cfg.source_branch) {
        return Err(PipelineError::Setup(format!(
            "source `{}` is not a local branch (tags, remotes, and HEAD are rejected)",
            cfg.source_branch
        )));
    }
    let source_commit = git::resolve_commit(&repo, &cfg.source_branch)?;

    // A caller-supplied slug is slugified too — never trusted verbatim into a
    // branch name / filesystem path (an unsanitised `../x` would traverse).
    let slug = cfg
        .slug
        .as_deref()
        .map_or_else(|| slugify(&cfg.intent), slugify);
    let integration_branch = format!("feat/{slug}");
    if git::branch_exists(&repo, &integration_branch) {
        return Err(PipelineError::Setup(format!(
            "integration branch `{integration_branch}` already exists; refusing to reuse it"
        )));
    }

    std::fs::create_dir_all(&cfg.workdir).map_err(|e| {
        PipelineError::Io(format!(
            "could not create workdir {}: {e}",
            cfg.workdir.display()
        ))
    })?;
    std::fs::write(cfg.workdir.join("intent.md"), &cfg.intent)
        .map_err(|e| PipelineError::Io(format!("could not write intent.md: {e}")))?;

    // Fork the integration branch from the pinned source OID (not the mutable
    // branch name), so the whole run is anchored to one commit even if the source
    // branch moves under us. The fork commit IS the source commit by construction.
    git::create_branch(&repo, &integration_branch, &source_commit)?;
    let integration_wt = cfg.workdir.join("integration");

    let mut run = Run {
        cfg,
        repo: repo.clone(),
        slug: slug.clone(),
        integration_branch: integration_branch.clone(),
        integration_wt: integration_wt.clone(),
        fork_commit: source_commit.clone(),
        decisions: Vec::new(),
        chunk_reports: Vec::new(),
        feature_floor: None,
        code_block_status: None,
        preserved: Vec::new(),
        merged_to_source: false,
    };

    git::worktree_add(&repo, &integration_wt, &integration_branch)?;
    let baseline_snapshot = capture_snapshot(cfg, &integration_wt)?;
    let baseline = BaselineSnapshot::new(format!("{integration_branch}@fork"), baseline_snapshot);

    // --- 2. Spec [Opus]: produce + validate the plan (retry once). ---
    let plan = produce_and_validate_plan(&run, spec, &baseline.to_plan_baseline())?;
    // Discard any side effect the (headless, permission-skipped) spec stage left
    // in the worktree: spec is a planner, so chunks must fork from a pristine
    // fork commit, not from spec's stray edits.
    git::restore_to(&run.integration_wt, &run.fork_commit)?;
    let plan_json = serde_json::to_string_pretty(&plan)
        .map_err(|e| PipelineError::Io(format!("could not serialize plan.json: {e}")))?;
    std::fs::write(cfg.workdir.join("plan.json"), &plan_json)
        .map_err(|e| PipelineError::Io(format!("could not write plan.json: {e}")))?;
    run.decisions.push(envelope(
        "spec",
        DecisionTier::Decider,
        format!("produced plan with {} chunk(s)", plan.chunks.len()),
        vec![
            format!("intent_rev:1"),
            format!("baseline:{}", baseline.r#ref),
        ],
        spec.model(),
        spec.prompt_version(),
    ));

    // --- 3. Code [claude-deepseek]: run each chunk, floor-gate, merge. ---
    run_code_stage(&mut run, &plan, code, &baseline)?;
    let code_ok = run.chunk_reports.iter().all(|c| c.merged);

    if !code_ok {
        // A chunk failed the floor / harness: v1 has no fix loop. Stop, preserve
        // the failing chunk, and report (the floor is the hard gate — no merge).
        // The specific terminal status was recorded by the code stage.
        let status = run.code_block_status.unwrap_or("chunk_failed");
        return Ok(finalize(&run, &plan, None, false, None, status));
    }

    // --- 4. Verify [Opus]: run acceptance checks + judge product-vs-intent. ---
    let verify_report = run_verify_stage(&mut run, &plan, verify)?;
    // The verify stage is a judge, but it too ran headless with skipped
    // permissions — restore the worktree to the exact floor-gated feature tip so
    // no verify-time mutation can smuggle content past the floor into the merge.
    let feat_tip = git::head(&run.integration_wt)?;
    git::restore_to(&run.integration_wt, &feat_tip)?;

    if !verify_report.passed {
        return Ok(finalize(
            &run,
            &plan,
            Some(verify_report),
            false,
            None,
            "verify_failed",
        ));
    }

    // --- 5. Merge: re-check the feature floor at the tip, then merge → source. ---
    let declared: Vec<PathBuf> = union_declared_files(&plan);
    let feature_floor = evaluate_feature_floor(&run, &plan, &baseline, &declared, &feat_tip)?;
    run.feature_floor = Some(feature_floor.clone());

    if !feature_floor.passed() {
        // Floor regressed at the tip — do NOT merge (design §4/§14).
        return Ok(finalize(
            &run,
            &plan,
            Some(verify_report),
            false,
            None,
            "floor_blocked",
        ));
    }

    // The consequential ship judgment (Opus-tier decider): the feature converged.
    run.decisions.push(envelope(
        "orchestrator",
        DecisionTier::Decider,
        "declared converged: verify passed and the feature floor is green",
        vec![
            format!("feat:{feat_tip}"),
            format!("source:{source_commit}"),
        ],
        verify.model(),
        verify.prompt_version(),
    ));

    // The merge mechanics are routine coordination (supervisor-tier), gated by
    // the decider decision above — kept as a SEPARATE envelope so the tier split
    // is honest (the merge is not itself an Opus judgment).
    match merge_feature_to_source(&run, &feat_tip)? {
        MergeOutcome::Merged { commit } => {
            run.merged_to_source = true;
            run.decisions.push(envelope(
                "supervisor",
                DecisionTier::Coordinator,
                format!("merged {feat_tip} into {}", run.cfg.source_branch),
                vec![
                    format!("feat:{feat_tip}"),
                    format!("source:{source_commit}"),
                ],
                "supervisor",
                "v1",
            ));
            Ok(finalize(
                &run,
                &plan,
                Some(verify_report),
                true,
                Some(commit),
                "merged",
            ))
        }
        MergeOutcome::Conflict { details } => {
            // The floor was green, but the source branch moved underneath us and
            // the merge conflicts. Report it (preserve the integration branch);
            // it is not a crash — the caller resolves and re-runs.
            let mut vr = verify_report;
            vr.summary = format!("{} (source merge conflicted: {details})", vr.summary);
            Ok(finalize(
                &run,
                &plan,
                Some(vr),
                false,
                None,
                "merge_conflict",
            ))
        }
    }
}

/// Ask the spec provider for a plan, normalize the authoritative fields
/// (feature/baseline/versions) over its output, validate with the T2 validator,
/// and retry once on an invalid plan (design §6 VAIHE 1). Returns the validated
/// [`Plan`].
fn produce_and_validate_plan(
    run: &Run,
    spec: &dyn SpecProvider,
    baseline: &plan::Baseline,
) -> Result<Plan, PipelineError> {
    let ctx = SpecContext {
        intent: &run.cfg.intent,
        slug: &run.slug,
        source_branch: &run.cfg.source_branch,
        integration_branch: &run.integration_branch,
        files: &run.cfg.files,
        worktree: &run.integration_wt,
        baseline,
    };

    let mut last_err: Option<PipelineError> = None;
    for attempt in 0..2 {
        let raw = spec.produce_plan(&ctx)?;
        let normalized = normalize_plan(raw, run, baseline);
        match plan::parse_and_validate_plan(&normalized) {
            Ok(p) => return Ok(p),
            Err(e) => {
                last_err = Some(PipelineError::PlanInvalid(format!(
                    "attempt {} of 2: {e}",
                    attempt + 1
                )));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| PipelineError::PlanInvalid("no plan produced".to_string())))
}

/// Overwrite the supervisor-owned fields on a spec-produced plan value so the
/// contract's identity/baseline/version fields are authoritative regardless of
/// what the model emitted — the model is trusted only for `chunks`/`acceptance`
/// (design §1: intent + baseline are orchestrator-owned, not spec-writable).
fn normalize_plan(
    raw: serde_json::Value,
    run: &Run,
    baseline: &plan::Baseline,
) -> serde_json::Value {
    use serde_json::json;
    let mut obj = match raw {
        serde_json::Value::Object(m) => m,
        // Not an object → leave it; the validator will reject it clearly.
        other => return other,
    };
    obj.insert(
        "schema_version".to_string(),
        json!(plan::PLAN_SCHEMA_VERSION),
    );
    obj.insert("plan_rev".to_string(), json!(1));
    obj.insert("intent_rev".to_string(), json!(1));
    obj.insert(
        "feature".to_string(),
        json!({
            "slug": run.slug,
            "source_branch": run.cfg.source_branch,
            "integration_branch": run.integration_branch,
        }),
    );
    obj.insert(
        "baseline".to_string(),
        json!({
            "ref": baseline.r#ref,
            "test_passlist_hash": baseline.test_passlist_hash,
            "clippy_warnings_hash": baseline.clippy_warnings_hash,
        }),
    );
    serde_json::Value::Object(obj)
}

/// The union of every chunk's `files_touched`, de-duplicated — the declared
/// scope the feature-level floor gates against.
fn union_declared_files(plan: &Plan) -> Vec<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    for chunk in &plan.chunks {
        for f in &chunk.files_touched {
            seen.insert(PathBuf::from(f));
        }
    }
    seen.into_iter().collect()
}

/// Run every chunk in dependency order: fork a chunk worktree off the current
/// integration tip, drive the code harness, apply the floor gates, and merge on
/// green (design §6 VAIHE 2). Stops at the first chunk that does not merge (v1
/// has no fix loop); the failing chunk's branch/worktree are preserved.
fn run_code_stage(
    run: &mut Run,
    plan: &Plan,
    code: &dyn CodeHarness,
    baseline: &BaselineSnapshot,
) -> Result<(), PipelineError> {
    let order = topo_order(&plan.chunks);
    for &idx in &order {
        let chunk = &plan.chunks[idx];
        let base_commit = git::head(&run.integration_wt)?;
        let chunk_branch = format!("{}/chunk-{}", run.slug, chunk.id);
        let chunk_wt = run.cfg.workdir.join(format!("chunk-{}", chunk.id));

        git::worktree_add_new_branch(&run.repo, &chunk_wt, &chunk_branch, &base_commit)?;

        let checks: Vec<HarnessCheck> = chunk
            .checks
            .iter()
            .enumerate()
            .map(|(i, c)| to_harness_check(i, c))
            .collect();
        let req = ChunkRequest {
            run_id: format!("pipeline-{}", run.slug),
            chunk_id: chunk.id.clone(),
            attempt_id: "a1".to_string(),
            worktree_path: chunk_wt.clone(),
            base_commit: base_commit.clone(),
            plan_rev: plan.plan_rev.to_string(),
            brief: chunk.brief.clone(),
            checks,
            files: chunk.files_touched.iter().map(PathBuf::from).collect(),
            timeout: run.cfg.chunk_timeout,
        };

        let cancel = CancelToken::new();
        let result = code
            .run_chunk(&req, &cancel)
            .map_err(|e| PipelineError::Harness(e.to_string()))?;

        match &result.outcome {
            ChunkOutcome::Committed { commit } => {
                // Validate the harness's claimed commit against real git state
                // BEFORE the floor — an adapter that lies (reports a commit but
                // left HEAD unmoved, or committed an empty/rewritten tree, or
                // left the passing work uncommitted) must not slip a merge past
                // the floor. The floor then gates the exact committed tree.
                let head = git::head(&chunk_wt)?;
                if &head != commit {
                    return block_and_stop(
                        run,
                        chunk,
                        "failed",
                        "chunk_failed",
                        format!("harness reported commit {commit} but worktree HEAD is {head}"),
                        &chunk_wt,
                        &chunk_branch,
                    );
                }
                if head == base_commit {
                    return block_and_stop(
                        run,
                        chunk,
                        "no_change",
                        "chunk_failed",
                        "harness reported a commit but HEAD did not advance".to_string(),
                        &chunk_wt,
                        &chunk_branch,
                    );
                }
                if !git::is_ancestor(&chunk_wt, &base_commit, &head)? {
                    return block_and_stop(
                        run, chunk, "failed", "chunk_failed",
                        format!("chunk commit {head} is not a descendant of its base {base_commit} (history rewritten)"),
                        &chunk_wt, &chunk_branch,
                    );
                }
                if !git::is_clean(&chunk_wt)? {
                    return block_and_stop(
                        run,
                        chunk,
                        "failed",
                        "chunk_failed",
                        "chunk worktree has uncommitted changes after the commit".to_string(),
                        &chunk_wt,
                        &chunk_branch,
                    );
                }
                let changed = floor::git::changed_files(&chunk_wt, &base_commit, &head)?;
                if changed.is_empty() {
                    return block_and_stop(
                        run,
                        chunk,
                        "no_change",
                        "chunk_failed",
                        "committed chunk has an empty diff".to_string(),
                        &chunk_wt,
                        &chunk_branch,
                    );
                }

                let verdict = gate_chunk(run, chunk, &chunk_wt, &base_commit, &changed, baseline)?;
                if verdict.passed() {
                    // Floor green → supervisor-side merge of the EXACT gated
                    // commit oid (not the mutable branch name — a stray child
                    // process could have advanced the branch after the gate).
                    let outcome = git::merge_no_ff(
                        &run.integration_wt,
                        &head,
                        &format!("pipeline: merge chunk {}", chunk.id),
                    )?;
                    match outcome {
                        MergeOutcome::Merged {
                            commit: merge_commit,
                        } => {
                            run.decisions.push(envelope(
                                "supervisor",
                                DecisionTier::Coordinator,
                                format!("chunk {} floor green — merged", chunk.id),
                                vec![format!("chunk:{}", chunk.id), format!("commit:{head}")],
                                "supervisor",
                                "v1",
                            ));
                            run.chunk_reports.push(ChunkReport {
                                id: chunk.id.clone(),
                                title: chunk.title.clone(),
                                tier: chunk.tier.wire_name().to_string(),
                                outcome: "committed".to_string(),
                                floor_passed: Some(true),
                                floor: Some(verdict),
                                merged: true,
                                commit: Some(head.clone()),
                                merge_commit: Some(merge_commit),
                                reason: None,
                                branch_preserved: None,
                            });
                            // Chunk branch is now merged into feat → safe to drop
                            // its worktree and branch.
                            let _ = git::worktree_remove(&run.repo, &chunk_wt);
                            let _ = git::delete_branch(&run.repo, &chunk_branch, false);
                        }
                        MergeOutcome::Conflict { details } => {
                            return block_and_stop(
                                run,
                                chunk,
                                "committed",
                                "chunk_merge_conflict",
                                format!("chunk merge conflict: {details}"),
                                &chunk_wt,
                                &chunk_branch,
                            )
                            .map(|()| record_chunk_floor(run, Some(verdict), Some(true)));
                        }
                    }
                } else {
                    // Floor blocked → preserve the chunk branch, record, stop.
                    run.decisions.push(envelope(
                        "supervisor",
                        DecisionTier::Coordinator,
                        format!("chunk {} floor blocked — preserved, not merged", chunk.id),
                        vec![format!("chunk:{}", chunk.id)],
                        "supervisor",
                        "v1",
                    ));
                    run.code_block_status = Some("chunk_floor_blocked");
                    push_blocked_chunk(
                        run,
                        chunk,
                        "committed",
                        Some(verdict),
                        Some(false),
                        "floor gate failed".to_string(),
                        &chunk_wt,
                        &chunk_branch,
                    );
                    return Ok(());
                }
            }
            ChunkOutcome::NoChange => {
                return block_and_stop(
                    run,
                    chunk,
                    "no_change",
                    "chunk_failed",
                    "chunk produced no commit".to_string(),
                    &chunk_wt,
                    &chunk_branch,
                );
            }
            ChunkOutcome::Failed { reason } => {
                return block_and_stop(
                    run,
                    chunk,
                    "failed",
                    "chunk_failed",
                    reason.clone(),
                    &chunk_wt,
                    &chunk_branch,
                );
            }
            ChunkOutcome::Timeout => {
                return block_and_stop(
                    run,
                    chunk,
                    "timeout",
                    "chunk_failed",
                    "chunk timed out".to_string(),
                    &chunk_wt,
                    &chunk_branch,
                );
            }
            ChunkOutcome::Cancelled => {
                return block_and_stop(
                    run,
                    chunk,
                    "cancelled",
                    "chunk_failed",
                    "chunk cancelled".to_string(),
                    &chunk_wt,
                    &chunk_branch,
                );
            }
        }
    }
    Ok(())
}

/// Record a blocked chunk with its terminal status and stop the code stage.
/// A thin wrapper over [`push_blocked_chunk`] that also stamps
/// [`Run::code_block_status`] so the driver reports the precise reason
/// (`chunk_failed` vs `chunk_merge_conflict`). Returns `Ok(())` (never an error)
/// so the many call sites can `return block_and_stop(…)` to end the loop.
#[allow(clippy::too_many_arguments, clippy::unnecessary_wraps)]
fn block_and_stop(
    run: &mut Run,
    chunk: &Chunk,
    outcome: &str,
    status: &'static str,
    reason: String,
    chunk_wt: &Path,
    chunk_branch: &str,
) -> Result<(), PipelineError> {
    run.code_block_status = Some(status);
    push_blocked_chunk(
        run,
        chunk,
        outcome,
        None,
        None,
        reason,
        chunk_wt,
        chunk_branch,
    );
    Ok(())
}

/// Backfill the floor verdict onto the just-pushed (blocked) chunk report — used
/// on the chunk-merge-conflict path, where the floor DID pass but the merge did
/// not, so the report should still carry the green floor verdict.
fn record_chunk_floor(run: &mut Run, floor: Option<FloorVerdict>, floor_passed: Option<bool>) {
    if let Some(last) = run.chunk_reports.last_mut() {
        last.floor = floor;
        last.floor_passed = floor_passed;
    }
}

/// Record a chunk that did not merge and mark its worktree/branch preserved for
/// inspection (state-integrity invariant 5).
#[allow(clippy::too_many_arguments)]
fn push_blocked_chunk(
    run: &mut Run,
    chunk: &Chunk,
    outcome: &str,
    floor: Option<FloorVerdict>,
    floor_passed: Option<bool>,
    reason: String,
    chunk_wt: &Path,
    chunk_branch: &str,
) {
    run.preserved
        .push((chunk_wt.to_path_buf(), chunk_branch.to_string()));
    run.chunk_reports.push(ChunkReport {
        id: chunk.id.clone(),
        title: chunk.title.clone(),
        tier: chunk.tier.wire_name().to_string(),
        outcome: outcome.to_string(),
        floor_passed,
        floor,
        merged: false,
        commit: None,
        merge_commit: None,
        reason: Some(reason),
        branch_preserved: Some(chunk_branch.to_string()),
    });
}

/// Evaluate the per-chunk floor (design §4): the chunk's own checks pass, no
/// baseline regression / new clippy / test-gaming, and the changed files stay in
/// scope. Test/clippy regressions are judged against the fork baseline; the
/// assertion-density signal is judged against the chunk's own **base commit**
/// (the current integration tip), not the fork — so a later chunk that guts a
/// test an earlier chunk added is caught, instead of hiding behind the fork's
/// lower count. File-scope is against the chunk's `files_touched`.
fn gate_chunk(
    run: &Run,
    chunk: &Chunk,
    chunk_wt: &Path,
    base_commit: &str,
    changed: &[PathBuf],
    baseline: &BaselineSnapshot,
) -> Result<FloorVerdict, PipelineError> {
    let check_results: Vec<CheckRun> = floor::runner::run_checks(&chunk.checks, chunk_wt);
    let current = capture_snapshot(run.cfg, chunk_wt)?;
    let declared: Vec<PathBuf> = chunk.files_touched.iter().map(PathBuf::from).collect();
    let baseline_assertions =
        floor::runner::assertion_counts_at_ref(&run.repo, base_commit, &declared)?;
    let current_assertions = floor::runner::assertion_counts_on_disk(chunk_wt, &declared);

    let inputs = FloorInputs {
        baseline: &baseline.snapshot,
        current: &current,
        check_results: &check_results,
        declared_files: &declared,
        changed_files: changed,
        baseline_assertions: &baseline_assertions,
        current_assertions: &current_assertions,
        file_scope_slack: run.cfg.file_scope_slack,
    };
    Ok(evaluate_floor(&inputs))
}

/// Run the plan's executable acceptance checks, then ask the verify provider to
/// judge product-vs-intent (design §6 VAIHE 3). Returns the verify report. The
/// feature-floor re-check re-runs the acceptance checks itself (on the pristine,
/// gated tip) rather than reusing these results, so a verify-time mutation can
/// never leave a stale-green acceptance result behind the final gate.
fn run_verify_stage(
    run: &mut Run,
    plan: &Plan,
    verify: &dyn VerifyProvider,
) -> Result<VerifyReport, PipelineError> {
    let acceptance_checks: Vec<plan::Check> = plan
        .acceptance
        .iter()
        .filter_map(acceptance_to_check)
        .collect();
    let acceptance_results = floor::runner::run_checks(&acceptance_checks, &run.integration_wt);
    let acceptance_checks_passed = acceptance_results.iter().all(|r| r.passed);

    let judgment: VerifyJudgment = verify.verify(&VerifyContext {
        intent: &run.cfg.intent,
        plan,
        worktree: &run.integration_wt,
        acceptance_results: &acceptance_results,
    })?;

    run.decisions.push(envelope(
        "verify",
        DecisionTier::Decider,
        format!(
            "acceptance checks {}, judge {}",
            if acceptance_checks_passed {
                "passed"
            } else {
                "FAILED"
            },
            if judgment.passed { "passed" } else { "FAILED" }
        ),
        vec![format!("plan:{}", plan.plan_rev)],
        verify.model(),
        verify.prompt_version(),
    ));

    Ok(VerifyReport {
        acceptance_checks_passed,
        judged_passed: judgment.passed,
        passed: acceptance_checks_passed && judgment.passed,
        summary: judgment.summary,
        findings: judgment.findings,
    })
}

/// Convert an executable `acceptance` item into a runnable [`plan::Check`];
/// LLM-judged `assertion` items have no command and yield `None`.
fn acceptance_to_check(a: &Acceptance) -> Option<plan::Check> {
    match a {
        Acceptance::Check {
            desc,
            run,
            cwd,
            expect_exit,
        } => Some(plan::Check {
            desc: desc.clone(),
            run: run.clone(),
            cwd: cwd.clone(),
            expect_exit: *expect_exit,
            extra: serde_json::Map::new(),
        }),
        Acceptance::Assertion { .. } => None,
    }
}

/// The feature-level floor re-check before the final merge (design §4: the floor
/// is re-checked at the tip). Scoped to the whole feature: the acceptance checks
/// are re-run FRESH on the current (restored, gated) tip, changed files are
/// `fork..feat`, declared files are the union. The assertion-density baseline is
/// the fork (the whole feature is judged against the pre-feature state).
fn evaluate_feature_floor(
    run: &Run,
    plan: &Plan,
    baseline: &BaselineSnapshot,
    declared: &[PathBuf],
    feat_tip: &str,
) -> Result<FloorVerdict, PipelineError> {
    let acceptance_checks: Vec<plan::Check> = plan
        .acceptance
        .iter()
        .filter_map(acceptance_to_check)
        .collect();
    let check_results = floor::runner::run_checks(&acceptance_checks, &run.integration_wt);
    let current = capture_snapshot(run.cfg, &run.integration_wt)?;
    let changed = floor::git::changed_files(&run.integration_wt, &run.fork_commit, feat_tip)?;
    let baseline_assertions =
        floor::runner::assertion_counts_at_ref(&run.repo, &run.fork_commit, declared)?;
    let current_assertions = floor::runner::assertion_counts_on_disk(&run.integration_wt, declared);
    let inputs = FloorInputs {
        baseline: &baseline.snapshot,
        current: &current,
        check_results: &check_results,
        declared_files: declared,
        changed_files: &changed,
        baseline_assertions: &baseline_assertions,
        current_assertions: &current_assertions,
        file_scope_slack: run.cfg.file_scope_slack,
    };
    Ok(evaluate_floor(&inputs))
}

/// Merge the exact floor-gated `feat_tip` oid into the source branch (design §6
/// VAIHE 4). Merges the OID (not the mutable branch name) in the worktree that
/// has the source branch checked out (verified clean) when there is one;
/// otherwise materializes a throwaway worktree, merges, and removes it. Returns
/// the merge [`MergeOutcome`] so the driver can report a conflict rather than
/// crash (the source branch may have moved after the floor turned green).
fn merge_feature_to_source(run: &Run, feat_tip: &str) -> Result<MergeOutcome, PipelineError> {
    let message = format!(
        "pipeline: merge {} into {}",
        run.integration_branch, run.cfg.source_branch
    );
    if let Some(src_wt) = git::worktree_for_branch(&run.repo, &run.cfg.source_branch)? {
        if !git::is_clean(&src_wt)? {
            return Err(PipelineError::Setup(format!(
                "source branch `{}` worktree {} is dirty; cannot merge",
                run.cfg.source_branch,
                src_wt.display()
            )));
        }
        git::merge_no_ff(&src_wt, feat_tip, &message)
    } else {
        // Source branch not checked out anywhere: materialize a scratch worktree.
        let src_wt = run.cfg.workdir.join("source-merge");
        git::worktree_add(&run.repo, &src_wt, &run.cfg.source_branch)?;
        let out = git::merge_no_ff(&src_wt, feat_tip, &message);
        let _ = git::worktree_remove(&run.repo, &src_wt);
        out
    }
}

/// Build the final report from the accumulated run state.
fn finalize(
    run: &Run,
    plan: &Plan,
    verify: Option<VerifyReport>,
    merged: bool,
    final_commit: Option<String>,
    status: &str,
) -> PipelineReport {
    let failure = match status {
        "merged" => None,
        "chunk_floor_blocked" => {
            Some("a chunk failed the deterministic floor; the feature was not merged".to_string())
        }
        "chunk_merge_conflict" => {
            Some("a chunk floor-passed but conflicted merging into the integration branch".to_string())
        }
        "chunk_failed" => {
            Some("a chunk failed to produce a mergeable commit (harness failure / no change / timeout)".to_string())
        }
        "verify_failed" => {
            Some("verify judged the product does not match intent (or an acceptance check failed); not merged".to_string())
        }
        "floor_blocked" => Some("the feature floor regressed at the tip; not merged".to_string()),
        "merge_conflict" => {
            Some("the feature floor was green but the source branch moved and the merge conflicted".to_string())
        }
        other => Some(other.to_string()),
    };
    PipelineReport {
        slug: run.slug.clone(),
        source_branch: run.cfg.source_branch.clone(),
        integration_branch: run.integration_branch.clone(),
        intent_rev: 1,
        plan_rev: plan.plan_rev,
        chunk_count: plan.chunks.len(),
        chunks: run.chunk_reports.clone(),
        verify,
        feature_floor: run.feature_floor.clone(),
        merged,
        final_commit,
        status: status.to_string(),
        decisions: run.decisions.clone(),
        failure,
    }
}

/// Tear down the scratch worktrees/branches (design §6 VAIHE 4 teardown), gated
/// on the terminal outcome (state-integrity invariant 5): the integration
/// worktree is always removed, but the integration branch is deleted only when
/// the feature merged to source; a preserved (unmerged) chunk keeps both its
/// worktree and branch. `--keep` skips teardown entirely.
fn teardown(run: &Run) {
    if run.cfg.keep {
        return;
    }
    // Remove the integration worktree (its work is merged or abandoned).
    let _ = git::worktree_remove(&run.repo, &run.integration_wt);

    // Delete the integration branch only when it holds no unmerged work: either
    // it merged to source (redundant), or — on an early failure (bad spec) — it
    // never accumulated a chunk commit beyond the source branch. If a chunk did
    // merge into it but the feature never reached source, PRESERVE it: those are
    // real, unmerged commits (state-integrity invariant 5, source-relative check).
    let safe_to_delete = run.merged_to_source
        || git::commits_ahead_of(&run.repo, &run.cfg.source_branch, &run.integration_branch)
            .is_ok_and(|n| n == 0);
    if safe_to_delete {
        // `-d` refuses to drop an unmerged branch, so even a race here fails
        // closed rather than losing work.
        let _ = git::delete_branch(&run.repo, &run.integration_branch, run.merged_to_source);
    }
    // Preserved chunk worktrees/branches are intentionally left in place — they
    // hold unmerged work (invariant 5). Nothing else to remove.
}

// --- CLI entry --------------------------------------------------------------

/// Parsed `pipeline run` arguments, kept independent of clap so the wiring can
/// be exercised without the parser.
pub struct PipelineRunConfig {
    /// The raw `--intent` value (string or file path / `@file`).
    pub intent: String,
    /// The source branch.
    pub source_branch: String,
    /// Optional file-scope hints.
    pub files: Vec<PathBuf>,
    /// Optional slug override.
    pub slug: Option<String>,
    /// Optional repo path (default: cwd).
    pub repo: Option<PathBuf>,
    /// Optional test-capture command override.
    pub test_cmd: Option<String>,
    /// Optional clippy-capture command override.
    pub clippy_cmd: Option<String>,
    /// Optional workdir override.
    pub workdir: Option<PathBuf>,
    /// File-scope slack.
    pub file_scope_slack: usize,
    /// Keep worktrees after the run.
    pub keep: bool,
    /// Optional per-chunk timeout (seconds).
    pub chunk_timeout_secs: Option<u64>,
}

/// `pipeline run` entry point: resolve config, wire the LIVE Claude/deepseek
/// stages (design §10: spec/verify = `claude` Opus, code = `claude-deepseek`),
/// run the pipeline, and emit the report envelope.
pub fn cmd_run(
    cfg: &PipelineRunConfig,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let intent = resolve_intent(&cfg.intent)?;
    let repo = cfg
        .repo
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let slug_preview = cfg.slug.clone().unwrap_or_else(|| slugify(&intent));
    let workdir = cfg.workdir.clone().unwrap_or_else(|| {
        std::env::temp_dir()
            .join("octl-pipeline")
            .join(&slug_preview)
    });

    let pcfg = PipelineConfig {
        repo,
        intent,
        source_branch: cfg.source_branch.clone(),
        files: cfg.files.clone(),
        slug: cfg.slug.clone(),
        test_cmd: cfg
            .test_cmd
            .clone()
            .unwrap_or_else(|| "cargo test".to_string()),
        clippy_cmd: cfg
            .clippy_cmd
            .clone()
            .unwrap_or_else(|| "cargo clippy --message-format=short".to_string()),
        workdir,
        file_scope_slack: cfg.file_scope_slack,
        keep: cfg.keep,
        chunk_timeout: cfg.chunk_timeout_secs.map(Duration::from_secs),
    };

    // LIVE stages: spec/verify on ambient-login `claude` (Opus), code on the
    // `claude-deepseek` adapter (self-sources its deepseek key via SOPS — no
    // secret is read or hardcoded here; design §10).
    let spec_provider = providers::ClaudeSpecProvider;
    let verify_provider = providers::ClaudeVerifyProvider;
    let code = crate::harness::claude::ClaudeHarness::deepseek("flash");

    let report = run_pipeline(&pcfg, &spec_provider, &code, &verify_provider)?;

    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => output::emit_envelope(&report, spec, warnings)?,
        OutputFormat::Text => {
            print_report(&report);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

/// Render the human-readable pipeline summary (`--output text`).
fn print_report(r: &PipelineReport) {
    println!("pipeline {} — {}", r.slug, r.status);
    println!(
        "  source: {}  integration: {}",
        r.source_branch, r.integration_branch
    );
    println!("  chunks: {}", r.chunk_count);
    for c in &r.chunks {
        let floor = match c.floor_passed {
            Some(true) => "floor:green",
            Some(false) => "floor:BLOCKED",
            None => "floor:-",
        };
        let merged = if c.merged { "merged" } else { "not-merged" };
        println!(
            "    [{}] {} — {} {} {}",
            c.id, c.title, c.outcome, floor, merged
        );
        if let Some(reason) = &c.reason {
            println!("        reason: {}", output::escape_one_line(reason));
        }
    }
    if let Some(v) = &r.verify {
        println!(
            "  verify: {} (acceptance-checks: {}, judge: {}) — {}",
            if v.passed { "passed" } else { "FAILED" },
            v.acceptance_checks_passed,
            v.judged_passed,
            output::escape_one_line(&v.summary)
        );
    }
    match (&r.merged, &r.final_commit) {
        (true, Some(commit)) => println!("  merged → {} @ {}", r.source_branch, commit),
        _ => println!("  merged: no"),
    }
    if let Some(f) = &r.failure {
        println!("  failure: {}", output::escape_one_line(f));
    }
    for d in &r.decisions {
        println!(
            "  decision[{}] {}: {}",
            match d.decision_tier {
                DecisionTier::Coordinator => "coordinator",
                DecisionTier::Decider => "decider",
            },
            d.actor,
            output::escape_one_line(&d.reason)
        );
    }
}

#[cfg(test)]
mod tests;
