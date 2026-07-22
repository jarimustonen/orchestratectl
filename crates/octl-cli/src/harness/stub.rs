//! A deterministic, in-process fake [`CodeHarness`] the [`conformance`] suite
//! drives in CI with no network and no git (design.md §10: "provide a fake/stub
//! adapter that the conformance suite runs by default").
//!
//! The stub never shells out: given a [`StubBehavior`] it returns exactly the
//! scripted result or error, so the *contract* — the shape of every
//! [`ChunkResult`]/[`HarnessError`] — is tested independently of any real tool.
//! The live aider path is an opt-in smoke test; this is the always-on gate.
//!
//! [`conformance`]: super::conformance

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{
    CancelToken, Check, CheckResult, ChunkOutcome, ChunkRequest, ChunkResult, CodeHarness,
    HarnessCapabilities, HarnessError, Usage, HARNESS_CONTRACT_VERSION,
};

/// How often [`StubBehavior::SlowUntilCancel`] polls the cancel token while
/// counting down its budget (mirrors the real adapter's cooperative polling).
const STUB_POLL: Duration = Duration::from_millis(5);

/// What a [`StubHarness`] should return for a `run_chunk` call.
#[derive(Debug, Clone)]
pub enum StubBehavior {
    /// A completed run that committed `commit`, changed `changed_files`, and ran
    /// the request's checks (all pass unless `fail_first_check`).
    Commit {
        /// The resulting commit oid.
        commit: String,
        /// Files the "edit" touched.
        changed_files: Vec<PathBuf>,
        /// If true, the first requested check reports a failure.
        fail_first_check: bool,
    },
    /// A completed run that changed nothing.
    NoChange,
    /// A completed run that failed to produce a usable result.
    Failed {
        /// Why it failed.
        reason: String,
    },
    /// A completed run stopped by a timeout.
    Timeout,
    /// A completed run cancelled by the supervisor.
    Cancelled,
    /// A cooperatively-cancellable "slow run": it polls the cancel token until
    /// either the token is tripped (→ [`ChunkOutcome::Cancelled`]) or `budget`
    /// wall-clock elapses first (→ [`ChunkOutcome::Timeout`]). Lets the
    /// conformance suite exercise *in-flight* cancellation (another thread trips
    /// the token) and timeout expiry deterministically, with no network and no
    /// real agent — modelling how a real adapter bounds a live run.
    SlowUntilCancel {
        /// Wall-clock ceiling before the simulated run "times out".
        budget: Duration,
    },
    /// The harness could not produce a result at all.
    Error(HarnessError),
}

/// A scripted [`CodeHarness`] for deterministic conformance testing.
#[derive(Debug, Clone)]
pub struct StubHarness {
    behavior: StubBehavior,
    capabilities: HarnessCapabilities,
}

impl StubHarness {
    /// A stub with the given behavior and default (fully-capable) capabilities.
    pub fn new(behavior: StubBehavior) -> Self {
        Self {
            behavior,
            capabilities: HarnessCapabilities {
                can_author_tests: true,
                reports_usage: true,
                honors_file_scope: true,
                runs_checks: true,
            },
        }
    }

    /// Override the reported capabilities (e.g. to exercise a `reports_usage:
    /// false` branch).
    pub fn with_capabilities(mut self, capabilities: HarnessCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Synthesize a `CheckResult` per requested check without executing it.
    fn synth_checks(&self, checks: &[Check], fail_first: bool) -> Vec<CheckResult> {
        if !self.capabilities.runs_checks {
            return Vec::new();
        }
        checks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let passed = !(fail_first && i == 0);
                CheckResult {
                    check_id: c.id.clone(),
                    desc: c.desc.clone(),
                    run: c.run.clone(),
                    passed,
                    exit_code: Some(i32::from(!passed)),
                    stdout: String::new(),
                    stderr: String::new(),
                }
            })
            .collect()
    }
}

/// A stopped-early result (timeout/cancel): no commit, no checks — matching the
/// contract for the `Timeout`/`Cancelled` outcomes.
fn stopped_result(outcome: ChunkOutcome) -> ChunkResult {
    ChunkResult {
        schema_version: HARNESS_CONTRACT_VERSION,
        outcome,
        resulting_commit: None,
        changed_files: Vec::new(),
        check_results: Vec::new(),
        transcript_ref: Some(PathBuf::from("stub-transcript.log")),
        usage: None,
    }
}

impl CodeHarness for StubHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        self.capabilities
    }

    fn run_chunk(
        &self,
        req: &ChunkRequest,
        cancel: &CancelToken,
    ) -> Result<ChunkResult, HarnessError> {
        // Honour a cancel tripped before the run starts, for *any* scripted
        // behaviour (except a hard `Error`) — proves the token is threaded and
        // lets a test assert `Cancelled` by pre-tripping it.
        if cancel.is_cancelled() && !matches!(self.behavior, StubBehavior::Error(_)) {
            return Ok(stopped_result(ChunkOutcome::Cancelled));
        }

        // A cooperatively-cancellable slow run: poll the token until it trips
        // (Cancelled) or the budget elapses (Timeout).
        if let StubBehavior::SlowUntilCancel { budget } = &self.behavior {
            // A zero budget means "times out immediately" — return deterministically
            // rather than relying on `Instant::now()` monotonic drift within the loop.
            if budget.is_zero() {
                return Ok(stopped_result(ChunkOutcome::Timeout));
            }
            // `checked_add` so an absurd budget cannot overflow `Instant` and panic.
            let deadline = Instant::now().checked_add(*budget);
            loop {
                if cancel.is_cancelled() {
                    return Ok(stopped_result(ChunkOutcome::Cancelled));
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Ok(stopped_result(ChunkOutcome::Timeout));
                }
                std::thread::sleep(STUB_POLL);
            }
        }

        let transcript_ref = Some(PathBuf::from("stub-transcript.log"));
        let usage = self.capabilities.reports_usage.then_some(Usage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            cost_usd: Some(0.0001),
        });

        let result = match &self.behavior {
            StubBehavior::Commit {
                commit,
                changed_files,
                fail_first_check,
            } => ChunkResult {
                schema_version: HARNESS_CONTRACT_VERSION,
                outcome: ChunkOutcome::Committed {
                    commit: commit.clone(),
                },
                resulting_commit: Some(commit.clone()),
                changed_files: changed_files.clone(),
                check_results: self.synth_checks(&req.checks, *fail_first_check),
                transcript_ref,
                usage,
            },
            StubBehavior::NoChange => ChunkResult {
                schema_version: HARNESS_CONTRACT_VERSION,
                outcome: ChunkOutcome::NoChange,
                resulting_commit: None,
                changed_files: Vec::new(),
                // A completed run still reports its self-check state (like aider,
                // which runs checks even on NoChange) — required so the
                // completeness invariant holds when `runs_checks` is true.
                check_results: self.synth_checks(&req.checks, false),
                transcript_ref,
                usage,
            },
            StubBehavior::Failed { reason } => ChunkResult {
                schema_version: HARNESS_CONTRACT_VERSION,
                outcome: ChunkOutcome::Failed {
                    reason: reason.clone(),
                },
                resulting_commit: None,
                changed_files: Vec::new(),
                check_results: self.synth_checks(&req.checks, true),
                transcript_ref,
                usage,
            },
            StubBehavior::Timeout => stopped_result(ChunkOutcome::Timeout),
            StubBehavior::Cancelled => stopped_result(ChunkOutcome::Cancelled),
            // Handled above with a cooperative poll loop; never reaches the match.
            StubBehavior::SlowUntilCancel { .. } => {
                unreachable!("SlowUntilCancel is handled before the behavior match")
            }
            StubBehavior::Error(e) => return Err(e.clone()),
        };
        Ok(result)
    }
}
