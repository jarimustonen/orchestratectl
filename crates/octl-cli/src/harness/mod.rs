//! Harness-neutral code-executor contract (design.md §10, §5 harness-neutral
//! principle; build order §16 task 0).
//!
//! The code-pipeline supervisor must drive a code-writing agent (any
//! model/tool) over one *chunk* and consume a **structured** result — never
//! tool-specific prose, never exit-status guessing. This module defines that
//! seam:
//!
//! - [`CodeHarness`] — the versioned trait a concrete adapter implements.
//! - [`ChunkRequest`] / [`ChunkResult`] — the request/result protocol the
//!   supervisor speaks. `ChunkResult` is the *only* thing the supervisor reads
//!   back; an adapter that leaks tool prose or infers success from an exit code
//!   is non-conforming (design §10).
//! - [`HarnessError`] — structured failure modes (provider failure, malformed
//!   output, dirty worktree, …), never a stringly-typed blob.
//! - [`HarnessCapabilities`] — what an adapter supports, so the supervisor can
//!   branch (e.g. skip test-authoring when `can_author_tests` is false).
//!
//! Concrete adapters:
//! - [`aider::AiderHarness`] — the first conforming adapter (design §10: "First
//!   adapter: aider"). Shells out to `aider` non-interactively; commits but does
//!   NOT merge; maps the resulting *git* state → [`ChunkResult`] (never aider's
//!   stdout prose).
//! - [`stub::StubHarness`] — a deterministic in-process fake the [`conformance`]
//!   suite drives in CI with no network.
//!
//! **This module is behind the seam and not wired into any live path.** Nothing
//! in `run create` / the supervisor constructs a `CodeHarness` yet; staged
//! rollout (design §14) plugs it in later. It lands as unused-by-default
//! scaffolding + tests; the `mod harness;` declaration carries `#[allow(dead_code)]`
//! for exactly that reason.
//!
//! All protocol types are serde-serializable: they are recorded as run
//! provenance (design §7 "provenance recorded on every chunk attempt", §10
//! "runtime binding recorded in execution events").

pub mod aider;
pub mod bakeoff;
pub mod claude;
pub mod conformance;
pub mod pi;
pub mod stub;
pub(crate) mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A cheap-to-clone, thread-safe cancellation signal the supervisor holds while a
/// chunk runs and trips to abort it (design §9 circuit-breakers — a cost/time
/// kill-switch cancels an in-flight chunk and gets [`ChunkOutcome::Cancelled`]
/// back). Cloning shares one underlying flag (`Arc<AtomicBool>`), so a
/// supervisor thread can [`cancel`](CancelToken::cancel) the same token the
/// adapter polls from another thread.
///
/// The signal is one-way and level-triggered: once tripped it stays tripped.
/// Adapters honour it cooperatively — they poll [`is_cancelled`](CancelToken::is_cancelled)
/// at their wait points (e.g. between subprocess poll intervals) rather than
/// being interrupted asynchronously.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// A fresh, un-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Trip the token. Idempotent; observable from every clone. `Release`/`Acquire`
    /// is the standard write-once-flag pairing: the store on `cancel` happens-before
    /// the `Acquire` load in [`is_cancelled`](CancelToken::is_cancelled) that
    /// observes it. (No stronger `SeqCst` total order is needed — the token
    /// publishes only this one boolean, nothing else to order against.)
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Whether [`cancel`](CancelToken::cancel) has been called on this token (or
    /// any clone of it).
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// Serialize `Option<Duration>` as an optional integer count of **milliseconds**,
/// rather than serde's default `Duration` struct (`{"secs":…,"nanos":…}`). A
/// millisecond integer is the readable, stable wire shape for a timeout in the
/// provenance JSON — settled here before the contract is consumed by a live path.
/// Sub-millisecond precision is irrelevant for a wall-clock ceiling.
mod opt_duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    // `&Option<_>` is the signature serde's `with` contract requires here.
    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(v: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        v.map(|d| d.as_millis() as u64).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_millis))
    }
}

/// Version of the request/result protocol in this module. Bumped when the wire
/// shape of [`ChunkRequest`]/[`ChunkResult`] changes incompatibly, so recorded
/// provenance can be read against the schema it was written under (design §13
/// governed schema evolution).
pub const HARNESS_CONTRACT_VERSION: u32 = 1;

/// A code executor the supervisor can drive over one chunk.
///
/// Implementations are chosen behind this versioned interface — no runtime is
/// crowned by a single spike (design §10 harness-neutral principle). The
/// supervisor consumes only [`ChunkResult`]; it never parses tool-specific prose
/// or infers success from an exit status.
///
/// `Send + Sync`: the supervisor will drive chunks concurrently and share the
/// harness as `Arc<dyn CodeHarness>` across threads. Adapters keep per-call
/// mutation behind interior mutability — the harness value carries no per-chunk
/// state, so one `Arc<dyn CodeHarness>` is safely shared across all in-flight
/// chunks (the per-call [`ChunkRequest`]/[`CancelToken`] carry everything a chunk
/// needs). This resolves the concurrency-model question raised in issue
/// `outright-tasty-son`: shared `Arc`, not per-call construction.
///
/// **Execution control.** `run_chunk` bounds each attempt two ways so a runaway
/// or hung agent cannot block the supervisor forever (design §9 circuit-breakers):
/// - [`ChunkRequest::timeout`] — an optional wall-clock ceiling on the agent
///   invocation. On expiry the adapter kills the agent's process group and
///   returns [`ChunkOutcome::Timeout`] (never a hang).
/// - `cancel: &CancelToken` — a supervisor-tripped signal. A circuit-breaker that
///   trips it aborts the in-flight run and gets [`ChunkOutcome::Cancelled`] back.
///
/// Both stops are cooperative and clean: the adapter kills the child process
/// group, drains its partial transcript, and returns a *completed*
/// `Ok(ChunkResult)` — not a `HarnessError`.
///
/// **Scope / known limitations (resolved in later build-order tasks, not here):**
/// - **Worktree state after a stop is undefined.** A killed agent may have left
///   uncommitted edits, untracked files, or even a commit before it was stopped.
///   A `Timeout`/`Cancelled` [`ChunkResult`] carries *no* commit (the contract
///   forbids it), so the supervisor MUST reset the worktree to `base_commit`
///   before retrying — the adapter does not roll back (it never destroys
///   evidence). Transactional isolation (a disposable nested worktree) is T5/T6.
/// - **Only the agent invocation and each check are bounded/cancellable.** The
///   short git-inspection tail after a successful agent run (`git rev-parse`,
///   `git diff`) is not routed through the deadline/cancel machinery; a wedged
///   git could still stall the tail. A chunk-wide deadline covering the tail and
///   the whole check phase is a design §9 circuit-breaker (breakdown T6), not
///   part of this per-attempt contract. [`ChunkRequest::timeout`] bounds the
///   agent phase; [`Check::timeout`] bounds each check independently.
pub trait CodeHarness: Send + Sync {
    /// What this adapter supports, so the supervisor can branch on it.
    fn capabilities(&self) -> HarnessCapabilities;

    /// Execute one chunk against the worktree in `req` and return a structured
    /// result. A `HarnessError` is returned only when no `ChunkResult` could be
    /// produced (provider failure, malformed output, dirty worktree, …); a run
    /// that *completed* — even one that changed nothing, failed its self-checks,
    /// timed out, or was cancelled — is an `Ok(ChunkResult)` whose
    /// [`ChunkResult::outcome`] carries the verdict.
    ///
    /// `cancel` is polled cooperatively during the (unbounded) agent invocation:
    /// when the supervisor trips it, the adapter kills the agent and returns
    /// [`ChunkOutcome::Cancelled`]. A [`ChunkRequest::timeout`] that expires
    /// yields [`ChunkOutcome::Timeout`] the same way. See the trait-level docs.
    fn run_chunk(
        &self,
        req: &ChunkRequest,
        cancel: &CancelToken,
    ) -> Result<ChunkResult, HarnessError>;
}

/// What an adapter supports, so the supervisor can branch its pipeline (design
/// §10: "so the supervisor can branch"). Additive over time; readers must
/// tolerate an adapter reporting less than they hoped for rather than assuming.
// A capability-flag struct: independent boolean feature switches are exactly the
// right shape here, so the >3-bools pedantic lint does not apply.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCapabilities {
    /// The adapter can author/modify tests as part of a chunk (design §4
    /// mandatory test-authoring). If false, the supervisor must route test work
    /// to a dedicated stage.
    pub can_author_tests: bool,
    /// The adapter reports token/cost usage in [`ChunkResult::usage`] (design §9
    /// cost instrumentation, §11 token discipline).
    pub reports_usage: bool,
    /// The adapter *guarantees* it confines edits to the declared
    /// [`ChunkRequest::files`] scope. Conservative: an adapter that merely passes
    /// the scope as a hint (aider gives the files as argv but can still create or
    /// edit other paths on instruction) reports `false` — "do not rely on me for
    /// scope." Either way the deterministic floor enforces file-scope at merge
    /// time (design §4); this flag never substitutes for that enforcement.
    pub honors_file_scope: bool,
    /// The adapter executes the request's [`ChunkRequest::checks`] itself and
    /// populates [`ChunkResult::check_results`] (the code-node self-check, design
    /// §3). If false, `check_results` is empty and the supervisor runs the checks.
    pub runs_checks: bool,
}

/// One executable acceptance/self check: a human description plus a shell
/// command the harness or supervisor can run. The mechanical, injection-resistant
/// half of the plan's criteria (design §4 `checks` vs `assertions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Stable identifier for this check within the chunk, so a [`CheckResult`]
    /// can be paired back to its `Check` even when two checks share a `run`
    /// command or a check is renamed. Matching on `run` string alone is
    /// ambiguous; the id is the join key the conformance suite enforces.
    pub id: String,
    /// Human-readable description of what the check verifies.
    pub desc: String,
    /// Shell command executed via `sh -c` in the worktree. Exit 0 = pass.
    pub run: String,
    /// Optional wall-clock ceiling for this one check, so a wedged command (e.g.
    /// a `cargo test` that hangs) cannot stall the chunk. On expiry the adapter
    /// kills the check's process group and records it as a non-passing
    /// [`CheckResult`] with `exit_code: None` (design §9 resource safety). `None`
    /// = unbounded (but the harness-wide cancel still applies).
    #[serde(
        with = "opt_duration_millis",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
}

/// Everything an adapter needs to execute one chunk attempt. Harness-neutral: it
/// names *what* to do (brief + checks + scope) and *where* (worktree + base),
/// never *how* (model/tool/credentials — those live in the adapter's own config,
/// out of the plan, per design §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRequest {
    /// Run this chunk belongs to (provenance / causal id).
    pub run_id: String,
    /// Chunk id within the plan DAG.
    pub chunk_id: String,
    /// Attempt id — a chunk may be re-briefed and re-run (design §7 fix loop);
    /// each attempt records the exact `plan_rev` it consumed.
    pub attempt_id: String,
    /// Worktree the adapter edits in. The adapter must NOT merge; it commits its
    /// chunk branch (design §3 code role).
    pub worktree_path: PathBuf,
    /// Git object id the chunk forks from. Used to detect the resulting commit
    /// and compute `changed_files` against — from git, not tool prose.
    pub base_commit: String,
    /// The immutable plan revision this attempt was briefed against (provenance).
    pub plan_rev: String,
    /// The turnkey instruction the code agent executes (design §3 spec role
    /// writes "turnkey briefs").
    pub brief: String,
    /// Executable checks the adapter runs as its self-check (design §4). At least
    /// one per chunk in a real plan; the type does not enforce that so an adapter
    /// can be exercised with none.
    pub checks: Vec<Check>,
    /// Declared file scope for this chunk (design §4 `files_touched[]`). Passed to
    /// adapters that can constrain their edits (aider takes these as its file
    /// args); the supervisor still enforces scope at merge time regardless.
    ///
    /// Beyond the minimal `{run_id, chunk_id, attempt_id, worktree, base_commit,
    /// plan_rev, brief, checks}` set the interface issue lists — added because a
    /// file-oriented adapter (aider) genuinely needs the edit scope, and design
    /// §4 already models it as `files_touched[]`.
    #[serde(default)]
    pub files: Vec<PathBuf>,
    /// Optional wall-clock ceiling for the agent invocation (design §9 wall-time
    /// circuit-breaker). When the agent exceeds it, the adapter kills its process
    /// group and returns [`ChunkOutcome::Timeout`] with the partial transcript.
    /// `None` = no ceiling (the adapter is then bounded only by cancellation).
    /// Recorded as provenance: what deadline this attempt ran under. Per-`Check`
    /// timeouts ([`Check::timeout`]) bound the self-check phase separately.
    #[serde(
        with = "opt_duration_millis",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
}

/// How a completed chunk attempt turned out. Every variant is a *completed* run
/// the supervisor consumes — failures that prevented producing a result are a
/// [`HarnessError`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChunkOutcome {
    /// The agent committed a change. `commit` is the resulting git oid and MUST
    /// equal [`ChunkResult::resulting_commit`].
    Committed {
        /// Resulting commit oid.
        commit: String,
    },
    /// The run completed but produced no commit (synthesized when the tool made
    /// no change — design §10 "Synthesize `NoChange` when no commit produced").
    NoChange,
    /// The run ran but failed to produce a usable result (e.g. the tool exited
    /// non-zero with no commit). Distinct from a `HarnessError`: the harness
    /// *did* drive the agent; the agent just did not succeed.
    Failed {
        /// Why the run failed, for triage.
        reason: String,
    },
    /// The run exceeded its allotted wall-clock and was stopped. Carries no
    /// commit even if the agent committed before the kill — the worktree state is
    /// undefined and the supervisor must reset before retrying (see the
    /// [`CodeHarness`] trait docs).
    Timeout,
    /// The run was cancelled (e.g. a supervisor circuit-breaker, design §9).
    /// Same worktree-reset caveat as [`ChunkOutcome::Timeout`].
    Cancelled,
}

/// The result of running one executable [`Check`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// The [`Check::id`] this result is for (the join key).
    pub check_id: String,
    /// Echoes [`Check::desc`].
    pub desc: String,
    /// Echoes [`Check::run`] (the command that was executed).
    pub run: String,
    /// Whether the command exited 0.
    pub passed: bool,
    /// Process exit code, if the command ran to completion (`None` if it could
    /// not be spawned or was killed by a signal).
    pub exit_code: Option<i32>,
    /// Captured stdout (may be truncated by the adapter).
    pub stdout: String,
    /// Captured stderr (may be truncated by the adapter).
    pub stderr: String,
}

/// Token/cost accounting for a chunk attempt, when the adapter can report it
/// (design §9 cost instrumentation). All fields optional — a provider that
/// reports only cost, only tokens, or nothing is representable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Input/prompt tokens.
    pub input_tokens: Option<u64>,
    /// Output/completion tokens.
    pub output_tokens: Option<u64>,
    /// Total tokens, when the provider reports a combined figure.
    pub total_tokens: Option<u64>,
    /// Cost in USD, when the provider reports it.
    pub cost_usd: Option<f64>,
}

/// The single structured value the supervisor reads back from a chunk attempt.
/// It never parses tool prose (design §10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkResult {
    /// Protocol version this result was produced under (provenance).
    pub schema_version: u32,
    /// How the run turned out.
    pub outcome: ChunkOutcome,
    /// The commit the attempt produced, if any. `Some` **iff** `outcome` is
    /// [`ChunkOutcome::Committed`] (see [`conformance::assert_result_conforms`]).
    pub resulting_commit: Option<String>,
    /// Files the attempt changed (`git diff --name-only base..HEAD`). Non-empty
    /// only when a commit was produced.
    pub changed_files: Vec<PathBuf>,
    /// Self-check results, when the adapter runs checks (`runs_checks`).
    pub check_results: Vec<CheckResult>,
    /// Reference to the captured transcript/log artifact, if any.
    pub transcript_ref: Option<PathBuf>,
    /// Token/cost usage, when reported.
    pub usage: Option<Usage>,
}

impl ChunkResult {
    /// A completed run that produced a commit.
    pub fn committed(commit: impl Into<String>, changed_files: Vec<PathBuf>) -> Self {
        let commit = commit.into();
        Self {
            schema_version: HARNESS_CONTRACT_VERSION,
            outcome: ChunkOutcome::Committed {
                commit: commit.clone(),
            },
            resulting_commit: Some(commit),
            changed_files,
            check_results: Vec::new(),
            transcript_ref: None,
            usage: None,
        }
    }

    /// A completed run that produced no commit.
    pub fn no_change() -> Self {
        Self {
            schema_version: HARNESS_CONTRACT_VERSION,
            outcome: ChunkOutcome::NoChange,
            resulting_commit: None,
            changed_files: Vec::new(),
            check_results: Vec::new(),
            transcript_ref: None,
            usage: None,
        }
    }

    /// A completed run that failed to produce a usable result.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            schema_version: HARNESS_CONTRACT_VERSION,
            outcome: ChunkOutcome::Failed {
                reason: reason.into(),
            },
            resulting_commit: None,
            changed_files: Vec::new(),
            check_results: Vec::new(),
            transcript_ref: None,
            usage: None,
        }
    }
}

/// Structured failure that prevented producing a [`ChunkResult`] at all.
///
/// A run that *completed* — with no change, a failed self-check, a timeout, or a
/// cancellation — is an `Ok(ChunkResult)`, not one of these. `HarnessError` is
/// reserved for "the harness could not drive the agent to a verdict."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HarnessError {
    /// The underlying provider/tool could not be run or failed transport-level
    /// (spawn error, non-recoverable API failure).
    #[error("provider failure: {message}")]
    ProviderFailure {
        /// Diagnostic detail.
        message: String,
    },
    /// The adapter produced output that could not be mapped to a `ChunkResult`
    /// (e.g. a router adapter got un-parseable JSON back).
    #[error("malformed harness output: {message}")]
    MalformedOutput {
        /// Diagnostic detail.
        message: String,
    },
    /// The worktree had uncommitted changes before the run; the adapter refuses
    /// to run rather than commingle prior edits with the chunk.
    #[error("dirty worktree: {details}")]
    DirtyWorktree {
        /// e.g. the `git status --porcelain` lines.
        details: String,
    },
    /// The worktree path was missing / not a git repo / not at `base_commit`.
    #[error("invalid worktree: {message}")]
    InvalidWorktree {
        /// Diagnostic detail.
        message: String,
    },
    /// A required credential/config was absent (e.g. `DEEPSEEK_API_KEY` unset).
    /// Names the missing variable, never its value.
    #[error("missing credential: environment variable `{var}` is not set")]
    MissingCredential {
        /// The environment variable that must be set.
        var: String,
    },
    /// The adapter itself hit an unexpected internal error (bug / I/O it could
    /// not classify).
    #[error("internal harness error: {message}")]
    Internal {
        /// Diagnostic detail.
        message: String,
    },
}
