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

use super::{
    Check, CheckResult, ChunkOutcome, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities,
    HarnessError, Usage, HARNESS_CONTRACT_VERSION,
};

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

impl CodeHarness for StubHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        self.capabilities
    }

    fn run_chunk(&self, req: &ChunkRequest) -> Result<ChunkResult, HarnessError> {
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
                check_results: Vec::new(),
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
            StubBehavior::Timeout => ChunkResult {
                schema_version: HARNESS_CONTRACT_VERSION,
                outcome: ChunkOutcome::Timeout,
                resulting_commit: None,
                changed_files: Vec::new(),
                check_results: Vec::new(),
                transcript_ref,
                usage: None,
            },
            StubBehavior::Cancelled => ChunkResult {
                schema_version: HARNESS_CONTRACT_VERSION,
                outcome: ChunkOutcome::Cancelled,
                resulting_commit: None,
                changed_files: Vec::new(),
                check_results: Vec::new(),
                transcript_ref,
                usage: None,
            },
            StubBehavior::Error(e) => return Err(e.clone()),
        };
        Ok(result)
    }
}
