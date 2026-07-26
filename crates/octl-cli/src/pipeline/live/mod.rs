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
//! # Scope
//!
//! Beyond the original happy-path skeleton, this now wires the **bounded
//! verify→triage→fix loop** (design §7 TRIGGER_RE_SPEC, §8 RE_CODE_CHUNK): a
//! floor-blocked chunk or a failed verify is fed back as a RE_CODE_CHUNK
//! re-brief and MUST re-verify before it can close; a SPEC-FLAW verdict emits
//! TRIGGER_RE_SPEC (a new `plan.v(N+1)` + a DAG-diff deciding which chunks revert
//! to Pending). The loop is bounded **hard** by the deterministic circuit-breakers
//! of [`fixloop::FixLoopConfig`] (design §9) so it can never loop on judgment
//! alone. With the breakers set to [`OFF`](fixloop::FixLoopConfig::OFF) the driver
//! reverts to the original walking-skeleton behaviour (the first failure is
//! terminal), which is how the pre-loop tests stay meaningful. The floor stays
//! the hard merge gate throughout (design §4/§14): a chunk or feature it blocks is
//! never merged.
//!
//! Tier promotion and the finer per-finding triage of design §8 (DISCUSS /
//! SPIN_OFF / DROP) remain deferred to follow-ups.

pub mod breakers;
pub mod fixloop;
pub mod git;
pub mod providers;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use octl_core::plan::{self, Acceptance, Chunk, Plan, Tier};
use serde::Serialize;

use crate::error::CliError;
use crate::floor::{
    self, evaluate_floor, BaselineSnapshot, CheckRun, FloorInputs, FloorVerdict, RunSnapshot,
};
use crate::harness::{CancelToken, Check as HarnessCheck, ChunkOutcome, ChunkRequest, CodeHarness};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::pipeline::{
    route_proposal, Action, ChunkState, ChunkStatus, Coordinator, CoordinatorProposal, Decider,
    DeciderVerdict, DecisionContext, DecisionEnvelope, DecisionTier, DecisionTrigger, Finding,
    FindingVerdict, Severity,
};

use breakers::{failure_fingerprint, ResourceBudget, ResourceMeter};
use fixloop::{next_tier, FixLoopConfig};
use git::MergeOutcome;
use providers::{
    SpecContext, SpecProvider, VerifyContext, VerifyDisposition, VerifyJudgment, VerifyProvider,
};

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
    /// Circuit-breaker bounds for the verify→triage→fix loop (design §9). Use
    /// [`FixLoopConfig::OFF`] for the v1 "first failure is terminal" behaviour.
    pub fix_loop: FixLoopConfig,
    /// Deterministic resource ceilings (design §9): cost/token, wall-time,
    /// process-count, storage, and repeated-identical-failure. Force the loop to
    /// abort regardless of convergence when crossed. Use
    /// [`ResourceBudget::UNLIMITED`] to disable every resource breaker.
    pub budget: ResourceBudget,
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
    /// Number of `RE_CODE_CHUNK` re-briefs performed across the whole run (design
    /// §8) — both code-stage floor re-codes and verify-driven fix re-codes.
    pub recode_count: u32,
    /// Number of `PROMOTE_TIER` promotions performed across the whole run (design
    /// §3): a repeat-failing chunk re-run at a higher model tier.
    pub promote_count: u32,
    /// Number of `TRIGGER_RE_SPEC` events (design §7). `plan_rev` equals `1 +
    /// respec_count`.
    pub respec_count: u32,
    /// Set when a deterministic circuit-breaker stopped the loop (design §9),
    /// naming which ceiling tripped. Its presence means the run terminated on a
    /// breaker rather than converging or hitting a plain terminal state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_breaker: Option<String>,
    /// The accumulated per-run resource tally (design §9 cost instrumentation):
    /// total tokens/cost metered from the harness [`Usage`], agent-invocation
    /// count, and peak scratch-storage bytes. Present on every run so the spend is
    /// auditable whether or not a breaker tripped.
    pub resources: ResourceMeter,
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

/// Resolves the [`CodeHarness`] a chunk runs on at a given [`Tier`] (design §3
/// adaptive promotion). `PROMOTE_TIER` re-runs a stuck chunk at a higher tier, so
/// the code stage selects its harness by the chunk's **current** (possibly
/// promoted) tier rather than a single fixed adapter.
pub trait TierHarness {
    /// The harness to run a chunk at `tier` on.
    fn harness(&self, tier: Tier) -> &dyn CodeHarness;

    /// The next tier up the ladder from `tier` that this resolver actually has a
    /// distinct harness for, or `None` at the ceiling — or when there is no ladder.
    /// Promotion consults THIS, not the abstract [`Tier`] enum, so a single-harness
    /// resolver never "promotes" a chunk onto the very same adapter (which would
    /// burn budget and mislabel the reported tier for no behavioural change).
    /// Defaults to the full `code → mid → high` ladder.
    fn next_tier(&self, tier: Tier) -> Option<Tier> {
        next_tier(tier)
    }
}

/// A [`TierHarness`] that returns ONE harness for every tier — the behaviour when
/// no per-tier ladder is configured (and what the 4-arg [`run_pipeline`] wraps its
/// single injected harness in). A promotion still bumps the recorded tier and
/// re-runs the chunk, but on the same adapter; the live command wires a real
/// per-tier ladder ([`LiveTierHarness`]) so a promoted chunk runs on a stronger
/// model, while unit tests use this to exercise the promotion control flow
/// deterministically.
pub struct SingleTierHarness<'a>(pub &'a dyn CodeHarness);

impl TierHarness for SingleTierHarness<'_> {
    fn harness(&self, _tier: Tier) -> &dyn CodeHarness {
        self.0
    }
    /// One harness for every tier means there is nothing to promote *to*: a
    /// single-harness resolver reports no higher tier, so `run_pipeline` (which
    /// wraps its one injected harness here) never promotes regardless of
    /// `max_promotions` — the pre-tiering behaviour is preserved exactly.
    fn next_tier(&self, _tier: Tier) -> Option<Tier> {
        None
    }
}

/// The live per-tier ladder (design §3/§10): cheap `claude-deepseek flash` for the
/// base tier, `claude-deepseek pro` for mid, and ambient Opus `claude` for high —
/// so a promoted chunk actually runs on a stronger model. Constructed by
/// [`cmd_run`]; the three adapters self-source their own credentials (no secret is
/// read here).
struct LiveTierHarness {
    code: crate::harness::claude::ClaudeHarness,
    mid: crate::harness::claude::ClaudeHarness,
    high: crate::harness::claude::ClaudeHarness,
}

impl TierHarness for LiveTierHarness {
    fn harness(&self, tier: Tier) -> &dyn CodeHarness {
        match tier {
            Tier::Code => &self.code,
            Tier::Mid => &self.mid,
            Tier::High => &self.high,
        }
    }
}

/// The live loop's fast **coordinator** (design §3 "coordinator (PM) … fast/cheap,
/// stateless fn"): the deterministic supervisor control flow *is* the coordinator.
/// It never *generates* proposals from context — the live loop already knows the
/// action each decision point implies — so [`coordinate`](Coordinator::coordinate)
/// is unused; the type exists only to carry the coordinator-tier envelope metadata
/// (actor / model / prompt version) into the shared [`route_proposal`] routing, so
/// routine live decisions are stamped by the SAME path the scaffold uses.
struct LiveCoordinator;

impl Coordinator for LiveCoordinator {
    fn coordinate(&self, _ctx: &DecisionContext) -> Vec<CoordinatorProposal> {
        Vec::new()
    }
    fn model(&self) -> String {
        "coordinator".to_string()
    }
    fn prompt_version(&self) -> String {
        "v1".to_string()
    }
}

/// A `'static` [`LiveCoordinator`] so the live loop can hold a `&'a dyn Coordinator`
/// without a lifetime shorter than the borrowed `cfg`. The coordinator is a ZST
/// with no state, so one shared instance is correct.
static LIVE_COORDINATOR: LiveCoordinator = LiveCoordinator;

/// The live **decider** seam (design §0.2/§2): the consequential-decision authority
/// the fast coordinator defers to. In the live loop the consequential proposals are
/// ALREADY backed by an Opus stage — `DECLARE_CONVERGED` ⟵ verify[Opus] passed +
/// the deterministic floor green, `TRIGGER_RE_SPEC` ⟵ verify[Opus]'s SPEC-FLAW
/// verdict + the Opus re-plan — so this decider **confirms** each proposal and
/// records Opus provenance, giving `decision_tier` an honest decider-tier stamp.
///
/// It is a deliberate seam, not a second Opus round-trip: it is where a distinct
/// second-opinion Opus decider drops in, and where the sequenced circuit-breaker
/// layer (`pipeline-circuit-breakers`) forces an `ESCALATE` override — the live
/// loop already honours a returned [`Action::Escalate`] at both consequential
/// decision points, so the breaker layer needs no further control-flow hook here.
struct LiveDecider {
    /// The Opus model backing the consequential decision (verify/spec are Opus in
    /// the live path), recorded on the decider-tier envelope.
    model: String,
}

impl Decider for LiveDecider {
    fn decide_consequential(
        &self,
        _ctx: &DecisionContext,
        proposed: &CoordinatorProposal,
    ) -> DeciderVerdict {
        DeciderVerdict {
            action: proposed.action.clone(),
            reason: proposed.reason.clone(),
            input_artifacts: proposed.input_artifacts.clone(),
        }
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn prompt_version(&self) -> String {
        "v1".to_string()
    }
}

/// Project the live run's chunk state into the [`DecisionContext`] the shared
/// [`route_proposal`] routing (and any decider it defers to) reads. The live loop
/// tracks a coarser [`LiveChunkStatus`]; map it into the scaffold's
/// [`ChunkStatus`], and carry each chunk's **current** (possibly promoted) tier so
/// a decider sees how far a chunk has already been escalated.
fn live_decision_ctx(run: &Run, plan: &Plan, trigger: DecisionTrigger) -> DecisionContext {
    let chunks = plan
        .chunks
        .iter()
        .map(|c| {
            let status = match run.chunk_status.get(&c.id) {
                Some(LiveChunkStatus::Merged) => ChunkStatus::AwaitingVerify,
                _ => ChunkStatus::Pending,
            };
            let tier = run.chunk_tier.get(&c.id).copied().unwrap_or(c.tier);
            (c.id.clone(), ChunkState { status, tier })
        })
        .collect();
    DecisionContext {
        run_id: format!("pipeline-{}", run.slug),
        plan_rev: plan.plan_rev,
        intent_rev: plan.intent_rev,
        chunks,
        trigger,
    }
}

/// Internal running state threaded through the driver's stages so teardown and
/// the report can see what was created.
struct Run<'a> {
    cfg: &'a PipelineConfig,
    /// The fast coordinator whose metadata stamps routine live decisions (design
    /// §3). A `'static` ZST — the live control flow supplies the proposals.
    coordinator: &'a dyn Coordinator,
    /// The decider the shared routing defers every *consequential* live decision to
    /// (design §0.2). Injected so tests can spy on / override it.
    decider: &'a dyn Decider,
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
    /// (`chunk_floor_blocked` vs `chunk_failed` vs `chunk_merge_conflict`, or
    /// `circuit_breaker` once a chunk's re-code budget is exhausted).
    code_block_status: Option<&'static str>,
    /// Per-chunk lifecycle across the fix loop (design §7). Seeded Pending from
    /// the plan; a chunk becomes `Merged` when it lands on `feat/<slug>`, and is
    /// reset to `Pending` by a `RE_CODE_CHUNK` re-brief or a re-spec DAG-diff.
    chunk_status: BTreeMap<String, LiveChunkStatus>,
    /// Each chunk's **current** model tier (design §3). Seeded from the plan's
    /// declared `chunk.tier`; a `PROMOTE_TIER` bumps it up the ladder so the code
    /// stage re-runs the chunk on a stronger harness.
    chunk_tier: BTreeMap<String, Tier>,
    /// How many times each chunk has already been promoted (design §3), bounded by
    /// [`FixLoopConfig::max_promotions`].
    chunk_promotions: BTreeMap<String, u32>,
    /// Chunk (worktree, branch) pairs preserved because they were not merged.
    preserved: Vec<(PathBuf, String)>,
    /// Total `RE_CODE_CHUNK` re-briefs (design §8), for the report + breaker audit.
    recode_count: u32,
    /// Total `PROMOTE_TIER` promotions across the run (design §3), for the report.
    promote_count: u32,
    /// Total `TRIGGER_RE_SPEC` events (design §7).
    respec_count: u32,
    /// Set when a circuit-breaker stopped the loop (design §9).
    circuit_breaker: Option<String>,
    /// Live per-run resource accounting (design §9): tokens/cost metered from the
    /// harness [`Usage`], agent-invocation count, peak storage, and the
    /// identical-failure fingerprints. The deterministic breakers read this.
    meter: ResourceMeter,
    /// Wall-clock start, for the wall-time breaker (design §9). Held here so the
    /// breaker check is a pure function of a measured [`Duration`].
    started: Instant,
    merged_to_source: bool,
}

/// A chunk's lifecycle in the live fix loop (a coarse projection of design §7's
/// chunk states, sufficient for the skeleton). `NeedsReverify` is modelled by
/// resetting to `Pending`: a re-coded chunk is re-run *and* re-verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveChunkStatus {
    /// Not yet coded, reverted by a re-code / re-spec, or awaiting a re-run.
    Pending,
    /// Committed and merged into the integration branch.
    Merged,
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
/// This 4-arg form runs every chunk on the ONE injected `code` harness (no tier
/// ladder) and defers consequential decisions to a confirming in-process decider
/// — the pre-tiering behaviour. For adaptive tier promotion + a spy-able decider
/// seam use [`run_pipeline_tiered`].
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
    let resolver = SingleTierHarness(code);
    // The confirming decider preserves the pre-tiering behaviour: a consequential
    // decision is stamped decider-tier and its proposed action is recorded as-is.
    let decider = crate::pipeline::ScriptedDecider::confirming();
    run_pipeline_tiered(cfg, spec, &resolver, verify, &decider)
}

/// The tiered entry point (design §0.2/§3): runs each chunk on the tier the
/// `harnesses` resolver picks for its **current** (possibly promoted) tier, and
/// defers every consequential decision to the injected `decider`. A repeat-failing
/// chunk is re-run at a higher tier (`PROMOTE_TIER`) before the repeated-failure
/// breaker gives up. Routine decisions never touch the decider.
///
/// # Errors
///
/// As [`run_pipeline`].
pub fn run_pipeline_tiered(
    cfg: &PipelineConfig,
    spec: &dyn SpecProvider,
    harnesses: &dyn TierHarness,
    verify: &dyn VerifyProvider,
    decider: &dyn Decider,
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
        coordinator: &LIVE_COORDINATOR,
        decider,
        repo: repo.clone(),
        slug: slug.clone(),
        integration_branch: integration_branch.clone(),
        integration_wt: integration_wt.clone(),
        fork_commit: source_commit.clone(),
        decisions: Vec::new(),
        chunk_reports: Vec::new(),
        feature_floor: None,
        code_block_status: None,
        chunk_status: BTreeMap::new(),
        chunk_tier: BTreeMap::new(),
        chunk_promotions: BTreeMap::new(),
        preserved: Vec::new(),
        recode_count: 0,
        promote_count: 0,
        respec_count: 0,
        circuit_breaker: None,
        meter: ResourceMeter::new(),
        started: Instant::now(),
        merged_to_source: false,
    };

    git::worktree_add(&repo, &integration_wt, &integration_branch)?;
    let baseline_snapshot = capture_snapshot(cfg, &integration_wt)?;
    let baseline = BaselineSnapshot::new(format!("{integration_branch}@fork"), baseline_snapshot);

    // --- 2. Spec [Opus]: produce + validate the initial plan (retry once). ---
    let mut plan =
        produce_and_validate_plan(&mut run, spec, &baseline.to_plan_baseline(), 1, None)?;
    // (spec invocations are metered inside produce_and_validate_plan.)
    // Discard any side effect the (headless, permission-skipped) spec stage left
    // in the worktree: spec is a planner, so chunks must fork from a pristine
    // fork commit, not from spec's stray edits.
    git::restore_to(&run.integration_wt, &run.fork_commit)?;
    write_plan(&run, &plan)?;
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
    // Seed every chunk Pending (design §7): the code stage advances them to Merged.
    run.chunk_status = plan
        .chunks
        .iter()
        .map(|c| (c.id.clone(), LiveChunkStatus::Pending))
        .collect();
    // Seed each chunk's current tier from its plan-declared tier (design §3): a
    // PROMOTE_TIER bumps it up the ladder from here.
    run.chunk_tier = plan.chunks.iter().map(|c| (c.id.clone(), c.tier)).collect();

    // Per-chunk verify findings to fold into the next code-stage re-brief (design
    // §8 RE_CODE_CHUNK). Populated when a FIX verdict targets chunks; the code
    // stage consumes them, so it is cleared after each pass.
    let mut pending_findings: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // --- 3-5. The bounded verify→triage→fix loop (design §7/§8), stopped hard by
    //          the deterministic circuit-breakers of §9. ---
    let mut fix_iter = 0u32;
    let outcome = loop {
        // Deterministic resource breakers (design §9) at the round boundary, BEFORE
        // spending another code/verify cycle: refresh the storage measurement, then
        // trip on any crossed ceiling (cost/token/wall-time/process/storage).
        // Supervisor-owned — the orchestrator is never consulted about a breaker.
        refresh_storage(&mut run);
        if let Some(msg) = resource_breach(&run) {
            run.circuit_breaker = Some(msg);
            break LoopExit::Terminal {
                verify: None,
                status: "circuit_breaker",
            };
        }

        // CODE STAGE over the Pending chunks, each with its own bounded RE_CODE
        // re-brief loop (design §8). Already-Merged chunks are skipped.
        run_code_stage(&mut run, &plan, harnesses, &baseline, &pending_findings)?;
        pending_findings.clear();
        if run.circuit_breaker.is_some() {
            // A chunk exhausted its re-code budget — the repeated-failure breaker
            // tripped (design §9). Stop; the failing chunk is preserved.
            break LoopExit::Terminal {
                verify: None,
                status: "circuit_breaker",
            };
        }
        if !all_merged(&run, &plan) {
            // A chunk could not be merged and re-code was off / not applicable:
            // terminal at the specific status the code stage recorded (the floor
            // is the hard gate — no merge, design §4/§14).
            let status = run.code_block_status.unwrap_or("chunk_failed");
            break LoopExit::Terminal {
                verify: None,
                status,
            };
        }

        // VERIFY on the feature tip (design §6 VAIHE 3). Capture the floor-gated
        // tip BEFORE verify runs, then restore to it afterwards — verify runs
        // headless with skipped permissions, so a verify-time commit or untracked
        // write must never become the tip and smuggle content past the floor
        // (`restore_to` hard-resets AND cleans untracked files).
        let feat_tip = git::head(&run.integration_wt)?;
        let (verify_report, disposition) = run_verify_stage(&mut run, &plan, verify)?;
        git::restore_to(&run.integration_wt, &feat_tip)?;
        // The verify invocation spent process/wall-time budget — check the breakers
        // BEFORE acting on its verdict, so a verify that crossed a ceiling aborts
        // rather than converging-and-merging or launching a re-spec (design §9).
        if let Some(msg) = resource_breach(&run) {
            run.circuit_breaker = Some(msg);
            break LoopExit::Terminal {
                verify: Some(verify_report),
                status: "circuit_breaker",
            };
        }
        if verify_report.passed {
            break LoopExit::Converged {
                verify: verify_report,
                feat_tip,
            };
        }

        // Verify failed → triage, bounded by the fix-iteration breaker (design §9).
        if fix_iter >= run.cfg.fix_loop.max_fix_iterations {
            // With the bound at 0 no fix was ever attempted → the v1 terminal
            // `verify_failed`; otherwise the loop tried and the breaker trips.
            let status = if run.cfg.fix_loop.max_fix_iterations == 0 {
                "verify_failed"
            } else {
                run.circuit_breaker = Some(format!(
                    "verify still failing after {} fix iteration(s)",
                    run.cfg.fix_loop.max_fix_iterations
                ));
                "circuit_breaker"
            };
            break LoopExit::Terminal {
                verify: Some(verify_report),
                status,
            };
        }
        fix_iter += 1;

        match disposition {
            VerifyDisposition::Fix | VerifyDisposition::FixChunks { .. } => {
                // v1 limitation (design §7 "RE_CODE re-commits on feat"): a
                // re-coded chunk forks off the current feat tip and commits a NEW
                // fix ON TOP of its prior merged work — the old commit is not
                // reverted first. The re-brief carries the findings, so an additive
                // fix converges; a fix that must *replace* prior output is not
                // fully modelled yet (a per-chunk revert is a follow-up).
                let targets = resolve_fix_targets(&disposition, &plan, &run);
                if targets.is_empty() {
                    // No chunk to re-code (e.g. nothing merged yet) — cannot make
                    // progress on a fix, so this is a terminal verify failure.
                    break LoopExit::Terminal {
                        verify: Some(verify_report),
                        status: "verify_failed",
                    };
                }
                for id in &targets {
                    record_recode_decision(
                        &mut run,
                        &plan,
                        id,
                        &verify_report.findings,
                        "verify FIX",
                    );
                    pending_findings.insert(id.clone(), verify_report.findings.clone());
                    run.chunk_status
                        .insert(id.clone(), LiveChunkStatus::Pending);
                }
                // Loop back → the code stage re-runs the reverted chunks, then the
                // loop re-verifies (design §8: FIX-class MUST re-verify before close).
            }
            VerifyDisposition::SpecFlaw { reason, chunk_ids } => {
                if run.respec_count >= run.cfg.fix_loop.max_respec {
                    // Re-spec off (0) → terminal verify failure; else breaker trip.
                    let status = if run.cfg.fix_loop.max_respec == 0 {
                        "verify_failed"
                    } else {
                        run.circuit_breaker = Some(format!(
                            "re-spec budget exhausted after {} re-spec(s)",
                            run.cfg.fix_loop.max_respec
                        ));
                        "circuit_breaker"
                    };
                    break LoopExit::Terminal {
                        verify: Some(verify_report),
                        status,
                    };
                }
                // TRIGGER_RE_SPEC (design §7): route the consequential decision to
                // the decider, produce plan.v(N+1), DAG-diff which chunks revert to
                // Pending, then loop back to the code stage. If the decider declined
                // the re-spec (ESCALATE override) the loop hands up instead.
                match trigger_re_spec(
                    &mut run,
                    spec,
                    &plan,
                    &reason,
                    &chunk_ids,
                    &verify_report.findings,
                    &baseline,
                )? {
                    ReSpecOutcome::Replanned(new_plan) => plan = *new_plan,
                    ReSpecOutcome::Escalated => {
                        break LoopExit::Terminal {
                            verify: Some(verify_report),
                            status: "escalated",
                        };
                    }
                }
                // Loop back → code stage re-runs reverted chunks, then re-verifies.
            }
        }
    };

    // Resolve the loop outcome into a report.
    let (verify_report, feat_tip) = match outcome {
        LoopExit::Terminal { verify, status } => {
            return Ok(finalize(&run, &plan, verify, false, None, status));
        }
        LoopExit::Converged { verify, feat_tip } => (verify, feat_tip),
    };

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

    // The consequential ship judgment: the fast coordinator PROPOSES
    // DECLARE_CONVERGED (verify passed + the deterministic floor is green) and the
    // shared tiered routing defers it to the decider (design §0.2/§2). The decider
    // CONFIRMS — or overrides to ESCALATE (the seam the circuit-breaker layer
    // forces later): an escalation stops short of the merge.
    // The trigger is the passed verify report (no outstanding findings) — the
    // evidence the ship decision rests on.
    let converge_ctx = live_decision_ctx(
        &run,
        &plan,
        DecisionTrigger::VerifyReport {
            report_id: format!("verify-plan-v{}", plan.plan_rev),
            findings: Vec::new(),
        },
    );
    let (converge_action, converge_env) = route_proposal(
        run.coordinator,
        run.decider,
        &converge_ctx,
        CoordinatorProposal {
            action: Action::DeclareConverged,
            reason: "declared converged: verify passed and the feature floor is green".to_string(),
            input_artifacts: vec![
                format!("feat:{feat_tip}"),
                format!("source:{source_commit}"),
            ],
        },
    );
    run.decisions.push(converge_env);
    if !matches!(converge_action, Action::DeclareConverged) {
        // The decider declined to ship (an ESCALATE override, or any non-converge
        // verdict): do NOT merge — the feature is handed up rather than landed.
        return Ok(finalize(
            &run,
            &plan,
            Some(verify_report),
            false,
            None,
            "escalated",
        ));
    }

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

/// The outcome of the bounded fix loop: either a terminal state (a breaker trip,
/// a floor block, or a failed verify with the fix budget spent) or convergence
/// (verify passed, ready to merge at `feat_tip`).
enum LoopExit {
    /// The loop stopped without converging; carries the report status and the
    /// verify report, if the loop reached verify.
    Terminal {
        verify: Option<VerifyReport>,
        status: &'static str,
    },
    /// Verify passed; merge the feature at this tip.
    Converged {
        verify: VerifyReport,
        feat_tip: String,
    },
}

/// Whether every chunk in the current plan is Merged (design §7). The fix loop
/// proceeds to verify only when the whole plan is on the integration branch.
fn all_merged(run: &Run, plan: &Plan) -> bool {
    plan.chunks
        .iter()
        .all(|c| run.chunk_status.get(&c.id) == Some(&LiveChunkStatus::Merged))
}

/// The chunks a FIX verdict should re-code (design §8). Only **merged** chunks
/// are eligible (verify runs after the whole plan is on `feat`, so a Pending
/// chunk is not a re-code target). An explicit, in-plan `FixChunks` list is
/// honoured verbatim (deduplicated, plan order); a bare `Fix` falls back to every
/// merged chunk — the coarse default, since the judge did not attribute the
/// failure. An explicit list that names ONLY unknown/unmerged chunks yields an
/// empty target set (the caller then treats it as a terminal verify failure)
/// rather than silently exploding to an all-chunk re-code from one bad id.
fn resolve_fix_targets(disp: &VerifyDisposition, plan: &Plan, run: &Run) -> Vec<String> {
    let is_merged = |id: &str| run.chunk_status.get(id) == Some(&LiveChunkStatus::Merged);
    let all_merged: Vec<String> = plan
        .chunks
        .iter()
        .filter(|c| is_merged(&c.id))
        .map(|c| c.id.clone())
        .collect();
    match disp {
        VerifyDisposition::FixChunks { chunk_ids } => {
            // Keep only merged, in-plan ids, in plan order, deduplicated. An empty
            // result is returned as-is (NOT widened to every chunk).
            plan.chunks
                .iter()
                .map(|c| c.id.clone())
                .filter(|id| is_merged(id) && chunk_ids.iter().any(|c| c == id))
                .collect()
        }
        _ => all_merged,
    }
}

/// Record a `RE_CODE_CHUNK` decision as the T4 [`Action`] primitive, routed
/// through the shared tiered seam ([`route_proposal`]). `RE_CODE_CHUNK` is
/// **routine**, so the coordinator emits it directly (coordinator-tier) and the
/// decider is **not** consulted (design §0.2 — the cost win). `findings` are the
/// verify/floor findings folded into the re-brief.
fn record_recode_decision(
    run: &mut Run,
    plan: &Plan,
    chunk_id: &str,
    findings: &[String],
    source: &str,
) {
    let action = Action::ReCodeChunk {
        chunk_id: chunk_id.to_string(),
        findings: findings
            .iter()
            .enumerate()
            .map(|(i, f)| Finding {
                id: format!("{chunk_id}-f{i}"),
                summary: f.clone(),
                verdict: FindingVerdict::Fix,
                severity: Severity::Medium,
            })
            .collect(),
    };
    let ctx = live_decision_ctx(
        run,
        plan,
        DecisionTrigger::ChunkCommitted {
            chunk_id: chunk_id.to_string(),
        },
    );
    let (_, env) = route_proposal(
        run.coordinator,
        run.decider,
        &ctx,
        CoordinatorProposal {
            action,
            reason: format!("re-code chunk {chunk_id} ({source})"),
            input_artifacts: vec![format!("chunk:{chunk_id}")],
        },
    );
    run.recode_count = run.recode_count.saturating_add(1);
    run.decisions.push(env);
}

/// The outcome of a `TRIGGER_RE_SPEC` attempt after the decider seam ruled on it.
enum ReSpecOutcome {
    /// The decider confirmed the re-spec; the loop continues on the new plan.
    /// Boxed — a [`Plan`] is large and the [`Escalated`](ReSpecOutcome::Escalated)
    /// variant carries nothing, so boxing keeps the enum small.
    Replanned(Box<Plan>),
    /// The decider overrode the re-spec with an ESCALATE (or any non-re-spec
    /// verdict): the loop hands the feature up instead of re-planning.
    Escalated,
}

/// The tier a repeat-failing chunk should be promoted to (design §3), or `None`
/// when it may not promote: its promotion budget ([`FixLoopConfig::max_promotions`])
/// is spent, or the resolver has no higher tier to run it on. Consulting the
/// resolver (not the abstract [`Tier`] enum) is what stops a single-harness
/// resolver from "promoting" onto the same adapter.
fn promotion_target(
    run: &Run,
    harnesses: &dyn TierHarness,
    chunk_id: &str,
    current_tier: Tier,
) -> Option<Tier> {
    let used = run.chunk_promotions.get(chunk_id).copied().unwrap_or(0);
    if used >= run.cfg.fix_loop.max_promotions {
        return None;
    }
    harnesses.next_tier(current_tier)
}

/// `PROMOTE_TIER` (design §3): bump a stuck chunk to `promoted`, record the routine
/// decision through the shared tiered seam (routine → coordinator-tier, the decider
/// is NOT consulted), and update the promotion bookkeeping. `promoted` is the tier
/// [`promotion_target`] returned.
fn promote_chunk(run: &mut Run, plan: &Plan, chunk_id: &str, current_tier: Tier, promoted: Tier) {
    let ctx = live_decision_ctx(
        run,
        plan,
        DecisionTrigger::ChunkCommitted {
            chunk_id: chunk_id.to_string(),
        },
    );
    let (_, env) = route_proposal(
        run.coordinator,
        run.decider,
        &ctx,
        CoordinatorProposal {
            action: Action::PromoteTier {
                chunk_id: chunk_id.to_string(),
                tier: promoted,
            },
            reason: format!(
                "promote chunk {chunk_id} {} → {} (repeat-fail)",
                current_tier.wire_name(),
                promoted.wire_name()
            ),
            input_artifacts: vec![format!("chunk:{chunk_id}")],
        },
    );
    run.decisions.push(env);
    run.chunk_tier.insert(chunk_id.to_string(), promoted);
    let used = run
        .chunk_promotions
        .entry(chunk_id.to_string())
        .or_insert(0);
    *used = used.saturating_add(1);
    run.promote_count = run.promote_count.saturating_add(1);
}

/// `TRIGGER_RE_SPEC` (design §7): record the decision, ask the spec provider for a
/// new plan revision against the flaw reason, DAG-diff old→new to decide which
/// chunks revert to Pending, apply that to the run's chunk state, persist the new
/// plan revision, and return it.
fn trigger_re_spec(
    run: &mut Run,
    spec: &dyn SpecProvider,
    old_plan: &Plan,
    reason: &str,
    forced: &[String],
    findings: &[String],
    baseline: &BaselineSnapshot,
) -> Result<ReSpecOutcome, PipelineError> {
    let new_rev = old_plan.plan_rev.saturating_add(1);

    // Route the consequential TRIGGER_RE_SPEC through the shared tiered seam: it is
    // deferred to the decider (design §0.2/§2), whose verdict is recorded. A
    // confirming decider ratifies it (the Opus re-plan below is the authority); an
    // ESCALATE override stops the re-spec before any new plan is produced. The
    // verify SPEC-FLAW findings are carried into the context as the evidence the
    // decider rules on (so the seam is not blind).
    let trigger_findings: Vec<Finding> = findings
        .iter()
        .enumerate()
        .map(|(i, f)| Finding {
            id: format!("respec-f{i}"),
            summary: f.clone(),
            verdict: FindingVerdict::SpecFlaw,
            severity: Severity::High,
        })
        .collect();
    let ctx = live_decision_ctx(
        run,
        old_plan,
        DecisionTrigger::VerifyReport {
            report_id: format!("respec-v{new_rev}"),
            findings: trigger_findings,
        },
    );
    let (action, env) = route_proposal(
        run.coordinator,
        run.decider,
        &ctx,
        CoordinatorProposal {
            action: Action::TriggerReSpec {
                reason: reason.to_string(),
                chunk_ids: forced.to_vec(),
            },
            reason: format!("re-spec to plan.v{new_rev}: {reason}"),
            input_artifacts: vec![format!("plan:{}", old_plan.plan_rev)],
        },
    );
    run.decisions.push(env);
    // Execute the decider's RECORDED verdict, not the original proposal: the decider
    // may soften/retarget the re-spec (a different reason or forced-chunk set), and
    // the pipeline must do what the audit trail says it did. A non-re-spec verdict
    // (ESCALATE override) hands the feature up before any new plan is produced.
    let (reason, forced): (String, Vec<String>) = match action {
        Action::TriggerReSpec { reason, chunk_ids } => (reason, chunk_ids),
        _ => return Ok(ReSpecOutcome::Escalated),
    };
    run.respec_count = run.respec_count.saturating_add(1);

    // Produce plan.v(N+1). The spec stage runs headless in the integration
    // worktree; restore it to the current tip afterward so a planner's stray edit
    // never bleeds into the code stage.
    let feat_tip = git::head(&run.integration_wt)?;
    let old_raw = serde_json::to_value(old_plan)
        .map_err(|e| PipelineError::Io(format!("could not serialize prior plan: {e}")))?;
    let new_plan = produce_and_validate_plan(
        run,
        spec,
        &baseline.to_plan_baseline(),
        new_rev,
        Some((&old_raw, reason.as_str())),
    )?;
    // (the re-spec invocation is metered inside produce_and_validate_plan.)
    git::restore_to(&run.integration_wt, &feat_tip)?;

    // DAG-diff old→new: which chunks revert to Pending vs. stay Done (design §7).
    let merged: BTreeSet<String> = run
        .chunk_status
        .iter()
        .filter(|(_, s)| **s == LiveChunkStatus::Merged)
        .map(|(id, _)| id.clone())
        .collect();
    let diff = fixloop::dag_diff(old_plan, &new_plan, &merged, &forced);

    // Rebuild the chunk-status map for the NEW plan. Removed chunks drop out (with
    // their reports); kept-done chunks stay Merged; everything else is Pending.
    let mut status = BTreeMap::new();
    for id in &diff.kept_done {
        status.insert(id.clone(), LiveChunkStatus::Merged);
    }
    for id in &diff.revert_to_pending {
        status.insert(id.clone(), LiveChunkStatus::Pending);
    }
    run.chunk_status = status;
    // Rebuild each chunk's tier for the NEW plan (design §3): a kept-done chunk
    // keeps whatever tier it converged at; a reverted / brand-new chunk resets to
    // the new plan's declared tier and its promotion count clears — the re-coded
    // chunk earns its own fresh promotion budget.
    let keep: BTreeSet<&str> = diff.kept_done.iter().map(String::as_str).collect();
    let mut new_tier = BTreeMap::new();
    let mut new_promotions = BTreeMap::new();
    for c in &new_plan.chunks {
        if keep.contains(c.id.as_str()) {
            let tier = run.chunk_tier.get(&c.id).copied().unwrap_or(c.tier);
            new_tier.insert(c.id.clone(), tier);
            if let Some(&n) = run.chunk_promotions.get(&c.id) {
                new_promotions.insert(c.id.clone(), n);
            }
        } else {
            new_tier.insert(c.id.clone(), c.tier);
        }
    }
    run.chunk_tier = new_tier;
    run.chunk_promotions = new_promotions;
    // Drop reports for chunks no longer in the plan (removed) or about to be
    // re-coded (reverted) — the re-run upserts a fresh report for the latter.
    run.chunk_reports.retain(|r| keep.contains(r.id.as_str()));

    write_plan(run, &new_plan)?;
    run.decisions.push(envelope(
        "spec",
        DecisionTier::Decider,
        format!(
            "re-spec plan.v{new_rev}: {} chunk(s) revert to pending, {} kept done",
            diff.revert_to_pending.len(),
            diff.kept_done.len()
        ),
        vec![format!("plan:{new_rev}"), format!("intent_rev:1")],
        spec.model(),
        spec.prompt_version(),
    ));
    Ok(ReSpecOutcome::Replanned(Box::new(new_plan)))
}

/// Persist the plan for a revision to the workdir: `plan.json` always (the
/// current plan) plus `plan.v{N}.json` for the revision, so the immutable
/// per-revision history is retained for audit (design §7).
fn write_plan(run: &Run, plan: &Plan) -> Result<(), PipelineError> {
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| PipelineError::Io(format!("could not serialize plan.json: {e}")))?;
    std::fs::write(run.cfg.workdir.join("plan.json"), &plan_json)
        .map_err(|e| PipelineError::Io(format!("could not write plan.json: {e}")))?;
    std::fs::write(
        run.cfg
            .workdir
            .join(format!("plan.v{}.json", plan.plan_rev)),
        &plan_json,
    )
    .map_err(|e| PipelineError::Io(format!("could not write plan revision: {e}")))?;
    Ok(())
}

/// Bounded number of spec attempts: the initial produce plus repair re-prompts.
/// Keeps the pre-existing count (design §6 VAIHE 1 — bounded re-spec).
const MAX_PLAN_ATTEMPTS: u32 = 2;

/// Filename the last invalid plan is persisted under (in the workdir) when the
/// spec stage exhausts its attempts, so a human can inspect what the model
/// actually produced.
const INVALID_PLAN_FILE: &str = "plan.invalid.json";

/// Ask the spec provider for a plan, normalize the authoritative fields
/// (feature/baseline/versions) over its output, and validate with the T2
/// validator. On a validation failure this runs a **repair loop** (design §6
/// VAIHE 1): it re-prompts the spec model with the exact validator error and the
/// invalid JSON it produced, so the model corrects precisely that error rather
/// than re-guessing blind. The parse stays strict — the driver never patches a
/// missing field server-side; the model must produce valid output. Bounded to
/// [`MAX_PLAN_ATTEMPTS`]; on exhaustion the last invalid plan is persisted to the
/// workdir ([`INVALID_PLAN_FILE`]) and the error surfaces the last validator
/// message. Returns the validated [`Plan`].
fn produce_and_validate_plan(
    run: &mut Run,
    spec: &dyn SpecProvider,
    baseline: &plan::Baseline,
    plan_rev: u32,
    respec: Option<(&serde_json::Value, &str)>,
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

    // Carried across attempts so a repair re-prompt can feed back the exact
    // validator error and the raw JSON the model produced. `last_raw` holds the
    // model's own output (pre-normalize) — what it must correct — and is what we
    // persist on exhaustion.
    let mut last_raw: Option<serde_json::Value> = None;
    let mut last_err: Option<String> = None;

    for attempt in 0..MAX_PLAN_ATTEMPTS {
        // The first attempt produces the candidate (a re-spec if `respec` is set,
        // else a fresh plan); every later attempt is a VALIDATOR repair re-prompt
        // carrying the previous error + invalid JSON forward (after attempt 0 both
        // `last_raw` and `last_err` are always set). A re-spec that produces an
        // invalid plan is thus still repaired the same bounded way.
        let produced = match (attempt, &last_raw, &last_err) {
            (0, _, _) => match respec {
                Some((prev, reason)) => spec.respec_plan(&ctx, prev, reason),
                None => spec.produce_plan(&ctx),
            },
            (_, Some(invalid), Some(err)) => spec.repair_plan(&ctx, invalid, err),
            // Unreachable: fall back to a fresh produce rather than panic.
            _ => spec.produce_plan(&ctx),
        };
        // Count EVERY spec provider invocation toward the process-count breaker
        // (design §9) — including each validator-repair re-prompt, and even a call
        // that then errored. Metering here (rather than once at the call site) is
        // why the two callers no longer meter the spec stage themselves.
        run.meter.record_agent_run(None);
        let raw = match produced {
            Ok(raw) => raw,
            // The spec provider itself failed (spawn/timeout/parse). If a prior
            // attempt already produced an invalid plan, persist it so the failure
            // is still inspectable, then propagate the transport error.
            Err(e) => {
                let _ = persist_invalid_plan(run, last_raw.as_ref());
                return Err(e);
            }
        };
        let normalized = normalize_plan(raw.clone(), run, baseline, plan_rev);
        match plan::parse_and_validate_plan(&normalized) {
            Ok(p) => return Ok(p),
            Err(e) => {
                last_err = Some(e.to_string());
                last_raw = Some(raw);
            }
        }
    }

    // Exhausted: persist the last invalid plan so a human can inspect it (right
    // now nothing was kept), then fail with the last validator message.
    let last_err = last_err.unwrap_or_else(|| "no plan produced".to_string());
    let persisted = persist_invalid_plan(run, last_raw.as_ref());
    Err(PipelineError::PlanInvalid(format!(
        "spec produced an invalid plan after {MAX_PLAN_ATTEMPTS} attempt(s): {last_err}{persisted}"
    )))
}

/// Best-effort write of the last invalid plan to `<workdir>/plan.invalid.json`
/// and return a suffix naming the file for the error message (or a note that it
/// could not be persisted). Never fails the run — the primary error is the
/// validation failure, and losing the artifact must not mask it.
fn persist_invalid_plan(run: &Run, raw: Option<&serde_json::Value>) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let path = run.cfg.workdir.join(INVALID_PLAN_FILE);
    let body = serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());
    match std::fs::write(&path, body) {
        Ok(()) => format!(" (raw invalid plan written to {})", path.display()),
        Err(e) => format!(" (could not persist invalid plan: {e})"),
    }
}

/// Overwrite the supervisor-owned fields on a spec-produced plan value so the
/// contract's identity/baseline/version fields are authoritative regardless of
/// what the model emitted — the model is trusted only for `chunks`/`acceptance`
/// (design §1: intent + baseline are orchestrator-owned, not spec-writable).
fn normalize_plan(
    raw: serde_json::Value,
    run: &Run,
    baseline: &plan::Baseline,
    plan_rev: u32,
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
    obj.insert("plan_rev".to_string(), json!(plan_rev));
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

/// Run the plan's still-Pending chunks in dependency order, each through its own
/// bounded `RE_CODE` re-brief loop (design §6 VAIHE 2 + §8). For each chunk: fork a
/// worktree off the current integration tip, drive the harness, gate the floor,
/// and merge on green. A floor-blocked / harness-failed attempt is re-briefed
/// with its findings and retried up to
/// [`max_recode_per_chunk`](FixLoopConfig::max_recode_per_chunk) times; once that
/// budget is exhausted the repeated-failure circuit-breaker (design §9) stops the
/// stage and the last failing attempt is preserved. Already-Merged chunks (from a
/// prior iteration or a re-spec's kept-done set) are skipped. On any block the
/// stage stops (the floor is the hard gate); the caller inspects
/// [`Run::circuit_breaker`] / [`Run::code_block_status`].
/// The first resource ceiling crossed so far, if any (design §9), as the
/// `circuit_breaker` message. Pure over the run's meter + measured wall-clock —
/// supervisor-owned, never gated on the orchestrator. Storage uses the last value
/// [`refresh_storage`] observed (refreshed at each round boundary).
fn resource_breach(run: &Run) -> Option<String> {
    run.meter.breach(&run.cfg.budget, run.started.elapsed())
}

/// Re-measure the scratch-workdir size into the meter (design §9 storage ceiling)
/// so the next [`resource_breach`] sees the current disk footprint. Called at each
/// round boundary — cheap enough there, too heavy to run per attempt. A no-op when
/// the storage breaker is off.
fn refresh_storage(run: &mut Run) {
    if run.cfg.budget.max_storage_bytes.is_some() {
        let bytes = breakers::dir_size_bytes(&run.cfg.workdir);
        run.meter.observe_storage_bytes(bytes);
    }
}

fn run_code_stage(
    run: &mut Run,
    plan: &Plan,
    harnesses: &dyn TierHarness,
    baseline: &BaselineSnapshot,
    pending_findings: &BTreeMap<String, Vec<String>>,
) -> Result<(), PipelineError> {
    let order = topo_order(&plan.chunks);
    for &idx in &order {
        let chunk = &plan.chunks[idx];
        if run.chunk_status.get(&chunk.id) == Some(&LiveChunkStatus::Merged) {
            continue; // already on feat/<slug> (kept-done or merged earlier)
        }

        // Seed the re-brief with any verify findings the fix loop routed to this
        // chunk (a verify-driven RE_CODE_CHUNK). This seed PERSISTS across
        // floor-retry attempts: a floor failure mid re-code appends its findings
        // rather than erasing the verify context (why the chunk is being re-coded
        // at all), so the model never "forgets" the original fix on attempt 2.
        let verify_seed: Vec<String> = pending_findings.get(&chunk.id).cloned().unwrap_or_default();
        let mut findings: Vec<String> = verify_seed.clone();
        // Two counters (see design §3 + §8): `recode` is the per-tier re-code
        // attempt (1-based) the re-code budget bounds — it RESETS to 1 on a
        // promotion so each tier gets its own fresh budget. `seq` is a monotonic
        // attempt id that NEVER resets; it names the attempt's worktree/branch, so
        // a promoted re-run can never collide with a superseded lower-tier attempt's
        // branch even if that attempt's cleanup failed (the names stay distinct).
        let mut recode = 1u32;
        let mut seq = 1u32;
        loop {
            let current_tier = run.chunk_tier.get(&chunk.id).copied().unwrap_or(chunk.tier);
            match attempt_chunk(
                run,
                plan,
                chunk,
                harnesses,
                current_tier,
                baseline,
                seq,
                &findings,
            )? {
                ChunkAttempt::Merged {
                    verdict,
                    commit,
                    merge_commit,
                } => {
                    run.decisions.push(envelope(
                        "supervisor",
                        DecisionTier::Coordinator,
                        format!("chunk {} floor green — merged", chunk.id),
                        vec![format!("chunk:{}", chunk.id), format!("commit:{commit}")],
                        "supervisor",
                        "v1",
                    ));
                    upsert_chunk_report(
                        run,
                        ChunkReport {
                            id: chunk.id.clone(),
                            title: chunk.title.clone(),
                            tier: current_tier.wire_name().to_string(),
                            outcome: "committed".to_string(),
                            floor_passed: Some(true),
                            floor: Some(verdict),
                            merged: true,
                            commit: Some(commit),
                            merge_commit: Some(merge_commit),
                            reason: None,
                            branch_preserved: None,
                        },
                    );
                    run.chunk_status
                        .insert(chunk.id.clone(), LiveChunkStatus::Merged);
                    // A resource ceiling the merge's own spend crossed stops the
                    // stage here (design §9). This is a post-attempt backstop, not an
                    // atomic gate: this chunk already landed on `feat/<slug>` (its
                    // work is preserved there — the feature never reaches source, and
                    // teardown keeps the integration branch), and further chunks/
                    // rounds are what the breaker prevents.
                    refresh_storage(run);
                    if let Some(msg) = resource_breach(run) {
                        run.circuit_breaker = Some(msg);
                        run.code_block_status = Some("circuit_breaker");
                        return Ok(());
                    }
                    break;
                }
                ChunkAttempt::Blocked {
                    outcome,
                    status,
                    reason,
                    findings: attempt_findings,
                    floor,
                    floor_passed,
                    recodable,
                    wt,
                    branch,
                } => {
                    // Deterministic resource circuit-breakers (design §9), checked
                    // BEFORE spending another attempt on this chunk — the whole point
                    // of §9 (never gated on the model's judgment). (a) repeated-
                    // identical-failure: the SAME block (chunk + status + findings)
                    // recurring to the ceiling aborts instead of grinding the re-code
                    // budget on an unchanging failure. (b) any resource ceiling the
                    // attempt just metered crossed (cost/token/process/wall-time). On
                    // either, preserve the attempt (state-integrity invariant 5) and
                    // stop the stage at a `circuit_breaker` terminal.
                    let fp = failure_fingerprint(
                        &chunk.id,
                        current_tier.wire_name(),
                        status,
                        &attempt_findings,
                    );
                    let recurrence = run.meter.record_failure(&fp);
                    // Refresh the storage measurement here too (not only at the round
                    // boundary) so intra-code-stage disk growth can trip the breaker
                    // before the next round; a no-op when the storage breaker is off.
                    refresh_storage(run);
                    if let Some(msg) = run
                        .cfg
                        .budget
                        .identical_failure_breach(recurrence)
                        .or_else(|| resource_breach(run))
                    {
                        run.circuit_breaker = Some(msg.clone());
                        run.code_block_status = Some("circuit_breaker");
                        run.decisions.push(envelope(
                            "supervisor",
                            DecisionTier::Coordinator,
                            format!(
                                "chunk {} stopped by circuit-breaker — preserved, not merged ({msg})",
                                chunk.id
                            ),
                            vec![format!("chunk:{}", chunk.id)],
                            "supervisor",
                            "v1",
                        ));
                        push_blocked_chunk(
                            run,
                            chunk,
                            outcome,
                            floor,
                            floor_passed,
                            reason,
                            &wt,
                            &branch,
                        );
                        return Ok(());
                    }

                    // Re-code while the chunk is re-codable and its per-tier budget
                    // holds (design §8). `recode` is 1-based, so re-code N is allowed
                    // iff N ≤ max_recode_per_chunk.
                    if recodable && recode <= run.cfg.fix_loop.max_recode_per_chunk {
                        record_recode_decision(
                            run,
                            plan,
                            &chunk.id,
                            &attempt_findings,
                            "floor re-code",
                        );
                        // The floor-failed attempt is superseded — drop its
                        // worktree + branch to make room for the re-brief (it is
                        // not mergeable work; the final failed attempt, if the
                        // budget runs out, IS preserved below).
                        let _ = git::worktree_remove(&run.repo, &wt);
                        let _ = git::delete_branch(&run.repo, &branch, true);
                        // Persist the verify seed, append this attempt's floor
                        // findings (see `verify_seed` above).
                        findings = verify_seed
                            .iter()
                            .cloned()
                            .chain(attempt_findings)
                            .collect();
                        recode += 1;
                        seq += 1;
                        continue;
                    }

                    // Re-code budget exhausted at this tier. Before giving up,
                    // adaptive promotion (design §3): a repeat-failing chunk is
                    // re-run at the NEXT model tier the resolver offers. Bounded by
                    // `max_promotions` and the top of the ladder; only a re-codable
                    // block promotes (a merge conflict is not the model's fault).
                    if let Some(promoted) = recodable
                        .then(|| promotion_target(run, harnesses, &chunk.id, current_tier))
                        .flatten()
                    {
                        promote_chunk(run, plan, &chunk.id, current_tier, promoted);
                        // The failed attempt at the old tier is superseded — drop it.
                        // The promoted re-run gets a fresh per-tier re-code budget
                        // (`recode = 1`) but a NEW monotonic `seq`, so its
                        // worktree/branch never collide with the just-dropped attempt.
                        let _ = git::worktree_remove(&run.repo, &wt);
                        let _ = git::delete_branch(&run.repo, &branch, true);
                        findings = verify_seed
                            .iter()
                            .cloned()
                            .chain(attempt_findings)
                            .collect();
                        recode = 1;
                        seq += 1;
                        continue;
                    }

                    // Terminal for this chunk. Distinguish a breaker trip (we tried
                    // re-codes / promotions and exhausted them) from the v1
                    // first-failure block (re-code off, or a non-re-codable outcome
                    // like a conflict).
                    let promotions = run.chunk_promotions.get(&chunk.id).copied().unwrap_or(0);
                    if recodable && (run.cfg.fix_loop.max_recode_per_chunk > 0 || promotions > 0) {
                        run.circuit_breaker = Some(format!(
                            "chunk {} still blocked after {} attempt(s) and {promotions} promotion(s): {reason}",
                            chunk.id, seq
                        ));
                        run.code_block_status = Some("circuit_breaker");
                    } else {
                        run.code_block_status = Some(status);
                    }
                    run.decisions.push(envelope(
                        "supervisor",
                        DecisionTier::Coordinator,
                        format!(
                            "chunk {} blocked — preserved, not merged ({reason})",
                            chunk.id
                        ),
                        vec![format!("chunk:{}", chunk.id)],
                        "supervisor",
                        "v1",
                    ));
                    push_blocked_chunk(
                        run,
                        chunk,
                        outcome,
                        floor,
                        floor_passed,
                        reason,
                        &wt,
                        &branch,
                    );
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// The result of one chunk attempt: a green-floor merge, or a block (with the
/// findings a re-code would fold into the next brief, and whether the outcome is
/// re-codable at all — a merge conflict is not).
enum ChunkAttempt {
    /// Floor green and merged into the integration branch.
    Merged {
        /// The floor verdict at the gated commit.
        verdict: FloorVerdict,
        /// The chunk's own resulting commit oid.
        commit: String,
        /// The integration-branch merge commit.
        merge_commit: String,
    },
    /// The attempt did not merge; the worktree/branch are the (as-yet un-torn-down)
    /// attempt state.
    Blocked {
        /// Report `outcome` label (`committed`/`no_change`/`failed`/…).
        outcome: &'static str,
        /// Terminal status if this is the final attempt and re-code is off.
        status: &'static str,
        /// Human-readable block reason.
        reason: String,
        /// Findings folded into a re-code re-brief (floor violations, or the
        /// harness failure reason).
        findings: Vec<String>,
        /// The floor verdict, when the block came from the floor gate.
        floor: Option<FloorVerdict>,
        /// Whether the floor passed (`Some(true)` only on a merge conflict).
        floor_passed: Option<bool>,
        /// Whether this outcome can be retried by a re-code (a merge conflict
        /// cannot — re-coding the same chunk would not resolve a moved tip here).
        recodable: bool,
        /// The attempt's worktree (preserved on a terminal block).
        wt: PathBuf,
        /// The attempt's branch (preserved on a terminal block).
        branch: String,
    },
}

/// Run ONE attempt of a chunk: fork an attempt worktree off the current
/// integration tip, drive the harness with the (possibly re-briefed) brief,
/// validate the harness's claimed commit against real git state, gate the floor,
/// and — on green — merge the exact gated oid into the integration branch. Every
/// integrity check (lying commit, empty diff, rewritten history, dirty worktree)
/// is preserved from the pre-loop driver; a failure returns a re-codable
/// [`ChunkAttempt::Blocked`].
#[allow(clippy::too_many_arguments)]
fn attempt_chunk(
    run: &mut Run,
    plan: &Plan,
    chunk: &Chunk,
    harnesses: &dyn TierHarness,
    current_tier: Tier,
    baseline: &BaselineSnapshot,
    seq: u32,
    findings: &[String],
) -> Result<ChunkAttempt, PipelineError> {
    let base_commit = git::head(&run.integration_wt)?;
    // The first attempt (seq 1) keeps the bare chunk name; every later attempt —
    // re-code OR promotion — suffixes `-a{seq}` with the MONOTONIC sequence, so no
    // two attempts (across any tier) ever share a worktree/branch name, even if a
    // superseded attempt's cleanup failed.
    let suffix = if seq == 1 {
        String::new()
    } else {
        format!("-a{seq}")
    };
    let chunk_branch = format!("{}/chunk-{}{suffix}", run.slug, chunk.id);
    let chunk_wt = run.cfg.workdir.join(format!("chunk-{}{suffix}", chunk.id));

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
        attempt_id: format!("a{seq}"),
        worktree_path: chunk_wt.clone(),
        base_commit: base_commit.clone(),
        plan_rev: plan.plan_rev.to_string(),
        brief: fixloop::rebrief(&chunk.brief, findings),
        checks,
        files: chunk.files_touched.iter().map(PathBuf::from).collect(),
        timeout: run.cfg.chunk_timeout,
    };

    // Select the harness for the chunk's CURRENT tier (design §3): a promoted
    // chunk runs on the stronger adapter the resolver returns.
    let cancel = CancelToken::new();
    let result = harnesses
        .harness(current_tier)
        .run_chunk(&req, &cancel)
        .map_err(|e| PipelineError::Harness(e.to_string()))?;
    // Meter this agent invocation into the per-run tally (design §9 cost
    // instrumentation): fold in the harness-reported Usage and count the process.
    // Done for EVERY outcome — a timeout/cancel/no-change still spent tokens.
    run.meter.record_agent_run(result.usage.as_ref());

    // A harness failure (no change / failed / timeout / cancelled) is re-codable;
    // its reason is the sole re-brief finding.
    let harness_block = |outcome: &'static str, reason: String| ChunkAttempt::Blocked {
        outcome,
        status: "chunk_failed",
        findings: vec![reason.clone()],
        reason,
        floor: None,
        floor_passed: None,
        recodable: true,
        wt: chunk_wt.clone(),
        branch: chunk_branch.clone(),
    };

    let commit = match &result.outcome {
        ChunkOutcome::Committed { commit } => commit.clone(),
        ChunkOutcome::NoChange => {
            return Ok(harness_block(
                "no_change",
                "chunk produced no commit".to_string(),
            ))
        }
        ChunkOutcome::Failed { reason } => return Ok(harness_block("failed", reason.clone())),
        ChunkOutcome::Timeout => {
            return Ok(harness_block("timeout", "chunk timed out".to_string()))
        }
        ChunkOutcome::Cancelled => {
            return Ok(harness_block("cancelled", "chunk cancelled".to_string()))
        }
    };

    // Validate the harness's claimed commit against real git state BEFORE the
    // floor — an adapter that lies (reports a commit but left HEAD unmoved,
    // committed an empty/rewritten tree, or left the passing work uncommitted)
    // must not slip a merge past the floor. These are re-codable failures.
    let head = git::head(&chunk_wt)?;
    if head != commit {
        return Ok(harness_block(
            "failed",
            format!("harness reported commit {commit} but worktree HEAD is {head}"),
        ));
    }
    if head == base_commit {
        return Ok(harness_block(
            "no_change",
            "harness reported a commit but HEAD did not advance".to_string(),
        ));
    }
    if !git::is_ancestor(&chunk_wt, &base_commit, &head)? {
        return Ok(harness_block(
            "failed",
            format!(
                "chunk commit {head} is not a descendant of its base {base_commit} (history rewritten)"
            ),
        ));
    }
    if !git::is_clean(&chunk_wt)? {
        return Ok(harness_block(
            "failed",
            "chunk worktree has uncommitted changes after the commit".to_string(),
        ));
    }
    let changed = floor::git::changed_files(&chunk_wt, &base_commit, &head)?;
    if changed.is_empty() {
        return Ok(harness_block(
            "no_change",
            "committed chunk has an empty diff".to_string(),
        ));
    }

    let verdict = gate_chunk(run, chunk, &chunk_wt, &base_commit, &changed, baseline)?;
    if !verdict.passed() {
        // Floor blocked → re-codable, with the floor violations as findings.
        return Ok(ChunkAttempt::Blocked {
            outcome: "committed",
            status: "chunk_floor_blocked",
            reason: "floor gate failed".to_string(),
            findings: fixloop::floor_findings(&verdict),
            floor: Some(verdict),
            floor_passed: Some(false),
            recodable: true,
            wt: chunk_wt,
            branch: chunk_branch,
        });
    }

    // Floor green → supervisor-side merge of the EXACT gated commit oid (not the
    // mutable branch name — a stray child could have advanced the branch).
    match git::merge_no_ff(
        &run.integration_wt,
        &head,
        &format!("pipeline: merge chunk {}", chunk.id),
    )? {
        MergeOutcome::Merged {
            commit: merge_commit,
        } => {
            // Merged into feat → drop the chunk worktree + branch. Force-delete:
            // the branch is provably merged into the integration branch (its work
            // is preserved there), but `git branch -d` checks against the repo's
            // ambient HEAD — which is not `feat/<slug>` — and so would refuse and
            // leak the branch, colliding when a later fix-loop iteration re-runs
            // the same chunk.
            let _ = git::worktree_remove(&run.repo, &chunk_wt);
            let _ = git::delete_branch(&run.repo, &chunk_branch, true);
            Ok(ChunkAttempt::Merged {
                verdict,
                commit: head,
                merge_commit,
            })
        }
        MergeOutcome::Conflict { details } => Ok(ChunkAttempt::Blocked {
            outcome: "committed",
            status: "chunk_merge_conflict",
            reason: format!("chunk merge conflict: {details}"),
            findings: vec![format!("chunk merge conflict: {details}")],
            floor: Some(verdict),
            floor_passed: Some(true),
            recodable: false,
            wt: chunk_wt,
            branch: chunk_branch,
        }),
    }
}

/// Push a chunk report, replacing any existing report for the same chunk id
/// (a chunk can be re-run across fix-loop iterations — the latest outcome wins,
/// so the report stays one-per-chunk).
fn upsert_chunk_report(run: &mut Run, report: ChunkReport) {
    run.chunk_reports.retain(|r| r.id != report.id);
    run.chunk_reports.push(report);
}

/// Record a chunk that did not merge and mark its worktree/branch preserved for
/// inspection (state-integrity invariant 5). Upserts so a re-run's terminal block
/// replaces any earlier report for the chunk.
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
    upsert_chunk_report(
        run,
        ChunkReport {
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
        },
    );
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
) -> Result<(VerifyReport, VerifyDisposition), PipelineError> {
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
    // Count the verify agent invocation toward the process-count breaker (design
    // §9). Verify does not surface Usage through its trait, so no tokens/cost are
    // added — a documented follow-up (spec/verify token accounting).
    run.meter.record_agent_run(None);

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

    // The combined verdict is mechanical (acceptance checks) ∧ judged. The judge's
    // disposition (FIX vs SPEC-FLAW) only carries a signal when the JUDGE failed;
    // if the judge passed but an acceptance check failed, there is no SPEC-FLAW
    // signal, so fall back to a bare FIX (re-code, don't re-spec).
    let passed = acceptance_checks_passed && judgment.passed;
    let disposition = if judgment.passed {
        VerifyDisposition::Fix
    } else {
        judgment.disposition.clone()
    };

    // Build the re-brief findings. Feed the FAILED acceptance checks in as
    // mechanical findings (a judge-passed / check-failed verdict would otherwise
    // re-code with no context and just reproduce the same output → NoChange →
    // breaker). And guarantee at least one finding on any failure, so the
    // RE_CODE_CHUNK re-brief always differs from the original brief.
    let mut findings = judgment.findings;
    for r in acceptance_results.iter().filter(|r| !r.passed) {
        findings.push(format!("acceptance check failed: {} (`{}`)", r.desc, r.run));
    }
    if !passed && findings.is_empty() {
        findings.push(format!(
            "verify failed without specific findings: {}. Review the implementation against the intent and correct it.",
            summary_or_default(&judgment.summary)
        ));
    }

    Ok((
        VerifyReport {
            acceptance_checks_passed,
            judged_passed: judgment.passed,
            passed,
            summary: judgment.summary,
            findings,
        },
        disposition,
    ))
}

/// The judge summary, or a placeholder when it is blank — so a synthetic
/// fallback finding is never an empty sentence.
fn summary_or_default(summary: &str) -> &str {
    if summary.trim().is_empty() {
        "(no summary)"
    } else {
        summary
    }
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
        "escalated" => Some(
            "the decider escalated a consequential decision (declined to converge or re-spec) and handed the feature up; not merged — see the decision log for the reason".to_string(),
        ),
        "merge_conflict" => {
            Some("the feature floor was green but the source branch moved and the merge conflicted".to_string())
        }
        "circuit_breaker" => Some(
            run.circuit_breaker
                .clone()
                .unwrap_or_else(|| "a circuit-breaker stopped the fix loop; not merged".to_string()),
        ),
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
        recode_count: run.recode_count,
        promote_count: run.promote_count,
        respec_count: run.respec_count,
        circuit_breaker: run.circuit_breaker.clone(),
        resources: run.meter.clone(),
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
    /// Max `RE_CODE` re-attempts per chunk in the code stage (design §8/§9).
    /// `None` uses the live default.
    pub max_recode_per_chunk: Option<u32>,
    /// Max verify→fix cycles (design §8/§9). `None` uses the live default.
    pub max_fix_iterations: Option<u32>,
    /// Max `TRIGGER_RE_SPEC` events (design §7/§9). `None` uses the live default.
    pub max_respec: Option<u32>,
    /// Max `PROMOTE_TIER` promotions per chunk (design §3). `None` uses the live
    /// default; `0` disables adaptive promotion.
    pub max_promotions: Option<u32>,
    /// Resource circuit-breaker: cost ceiling in USD (design §9). `None` uses the
    /// live default; `0` disables the cost breaker.
    pub max_cost_usd: Option<f64>,
    /// Resource circuit-breaker: total-token ceiling (design §9). `None` uses the
    /// live default; `0` disables it.
    pub max_total_tokens: Option<u64>,
    /// Resource circuit-breaker: wall-time ceiling in seconds (design §9). `None`
    /// uses the live default; `0` disables it.
    pub max_wall_time_secs: Option<u64>,
    /// Resource circuit-breaker: max agent invocations (design §9). `None` uses the
    /// live default; `0` disables it.
    pub max_processes: Option<u32>,
    /// Resource circuit-breaker: scratch-storage ceiling in MiB (design §9). `None`
    /// uses the live default; `0` disables it.
    pub max_storage_mb: Option<u64>,
    /// Resource circuit-breaker: identical-failure recurrence ceiling (design §9).
    /// `None` uses the live default; `0` disables it.
    pub max_identical_failures: Option<u32>,
}

/// Resolve a `u64` resource ceiling from an optional CLI override: an explicit
/// `0` disables the breaker (`None`), any other value overrides, and an absent
/// flag falls back to `default` (design §9: `0` = off, uniform across breakers).
fn resolve_u64_ceiling(user: Option<u64>, default: Option<u64>) -> Option<u64> {
    match user {
        Some(0) => None,
        Some(v) => Some(v),
        None => default,
    }
}

/// [`resolve_u64_ceiling`] for a `u32` ceiling.
fn resolve_u32_ceiling(user: Option<u32>, default: Option<u32>) -> Option<u32> {
    match user {
        Some(0) => None,
        Some(v) => Some(v),
        None => default,
    }
}

/// [`resolve_u64_ceiling`] for a USD (`f64`) ceiling: only a finite, positive value
/// enables the cost breaker. A non-finite (`NaN`/`inf` — both of which `f64::parse`
/// accepts) or non-positive value disables it (`None`), never leaves it enabled but
/// impossible to trip. `PipelineRunConfig` is constructable from code, so this guard
/// is the authoritative one even though the CLI could also validate.
fn resolve_f64_ceiling(user: Option<f64>, default: Option<f64>) -> Option<f64> {
    match user {
        Some(v) if v.is_finite() && v > 0.0 => Some(v),
        Some(_) => None,
        None => default,
    }
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
        fix_loop: {
            // The verify→triage→fix loop is ON by default for the live command
            // (design §7/§8), bounded by the §9 breakers; each bound is
            // individually overridable.
            let d = FixLoopConfig::live_default();
            FixLoopConfig {
                max_recode_per_chunk: cfg.max_recode_per_chunk.unwrap_or(d.max_recode_per_chunk),
                max_fix_iterations: cfg.max_fix_iterations.unwrap_or(d.max_fix_iterations),
                max_respec: cfg.max_respec.unwrap_or(d.max_respec),
                max_promotions: cfg.max_promotions.unwrap_or(d.max_promotions),
            }
        },
        budget: {
            // The deterministic resource breakers are ON by default for the live
            // command (design §9), each ceiling individually overridable; a supplied
            // `0` disables that one breaker (`None` in the resolved budget). Wall-time
            // is expressed in seconds and storage in MiB on the CLI; both round-trip
            // through the resolve helper in those units, then convert.
            let d = ResourceBudget::live_default();
            ResourceBudget {
                max_cost_usd: resolve_f64_ceiling(cfg.max_cost_usd, d.max_cost_usd),
                max_total_tokens: resolve_u64_ceiling(cfg.max_total_tokens, d.max_total_tokens),
                max_wall_time: resolve_u64_ceiling(
                    cfg.max_wall_time_secs,
                    d.max_wall_time.map(|w| w.as_secs()),
                )
                .map(Duration::from_secs),
                max_processes: resolve_u32_ceiling(cfg.max_processes, d.max_processes),
                max_storage_bytes: resolve_u64_ceiling(
                    cfg.max_storage_mb,
                    d.max_storage_bytes.map(|b| b / (1024 * 1024)),
                )
                .map(|mb| mb.saturating_mul(1024 * 1024)),
                max_identical_failures: resolve_u32_ceiling(
                    cfg.max_identical_failures,
                    d.max_identical_failures,
                ),
            }
        },
    };

    // LIVE stages: spec/verify on ambient-login `claude` (Opus). The code stage is
    // a per-tier ladder (design §3/§10): cheap `claude-deepseek flash` at the base,
    // `claude-deepseek pro` at mid, and ambient Opus `claude` at high — so a
    // PROMOTE_TIER re-run actually escalates the model. Every adapter self-sources
    // its own credentials (no secret is read or hardcoded here).
    let spec_provider = providers::ClaudeSpecProvider;
    let verify_provider = providers::ClaudeVerifyProvider;
    use crate::harness::claude::ClaudeHarness;
    let harnesses = LiveTierHarness {
        code: ClaudeHarness::deepseek("flash"),
        mid: ClaudeHarness::deepseek("pro"),
        high: ClaudeHarness::claude(Some("opus".to_string())),
    };
    // The consequential-decision authority (design §0.2/§2): verify/spec are Opus in
    // the live path, so the decider records that provenance on decider-tier
    // envelopes. The routine coordination decisions never reach it.
    let decider = LiveDecider {
        model: verify_provider.model(),
    };

    let report = run_pipeline_tiered(
        &pcfg,
        &spec_provider,
        &harnesses,
        &verify_provider,
        &decider,
    )?;

    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => output::emit_envelope(&report, spec, warnings)?,
        OutputFormat::Text => {
            print_report(&report);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

/// Render a byte count in the largest binary unit that keeps it ≥ 1 (e.g. `2.0
/// GiB`, `512 B`), for the human-readable resource line.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
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
    if r.recode_count > 0 || r.respec_count > 0 || r.promote_count > 0 {
        println!(
            "  fix loop: {} re-code(s), {} promotion(s), {} re-spec(s) → plan.v{}",
            r.recode_count, r.promote_count, r.respec_count, r.plan_rev
        );
    }
    match (&r.merged, &r.final_commit) {
        (true, Some(commit)) => println!("  merged → {} @ {}", r.source_branch, commit),
        _ => println!("  merged: no"),
    }
    {
        let res = &r.resources;
        println!(
            "  resources: {} token(s), ${:.4}, {} agent invocation(s), {} scratch storage",
            res.total_tokens,
            res.cost_usd,
            res.processes,
            human_bytes(res.storage_bytes)
        );
    }
    if let Some(cb) = &r.circuit_breaker {
        println!("  circuit-breaker: {}", output::escape_one_line(cb));
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
