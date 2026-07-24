//! Reusable conformance suite for [`CodeHarness`] adapters (design.md §10: "A
//! conformance suite tests each adapter against: clean success, no-change,
//! self-check fail, malformed output, dirty worktree, transcript+usage capture,
//! …").
//!
//! The core is [`assert_result_conforms`] — the adapter-agnostic structural
//! invariants every [`ChunkResult`] must satisfy — and [`run_and_check`], which
//! runs an adapter and gates its result through those invariants. Every
//! git-inspecting adapter ([`aider`], [`claude`], [`pi`]) is "run through the
//! suite" by driving its scenarios via [`run_and_check`] in its own module's
//! tests — each against a **fixture script** (`OCTL_*_BIN` override) so the
//! contract cases (clean success, no-change, self-check fail, timeout/cancel,
//! provider spawn failure, transcript+usage capture) run deterministically with
//! no network. The [`StubHarness`] scenario matrix below is the tool-independent
//! core of that same contract.
//!
//! The scenario matrix runs against the [`StubHarness`] by default (no network,
//! no git) so CI tests the *contract* deterministically. Tests that drive a
//! **real** agent (not a fixture script) are gated behind the `OCTL_HARNESS_LIVE`
//! env var — see [`live_enabled`] and the live smoke tests below — so CI never
//! reaches for a binary/credential/network; the `harness bakeoff` command is the
//! primary live exercise and is run explicitly.
//!
//! [`stub`]: super::stub
//! [`aider`]: super::aider
//! [`claude`]: super::claude
//! [`pi`]: super::pi
//! [`StubHarness`]: super::stub::StubHarness

use std::collections::HashSet;

use super::{
    ChunkOutcome, ChunkRequest, ChunkResult, HarnessCapabilities, HARNESS_CONTRACT_VERSION,
};

/// A full git object id is 40 (SHA-1) or 64 (SHA-256) hex chars.
fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Assert a [`ChunkResult`] satisfies the structural contract every adapter must
/// honour, independent of which tool produced it. `caps` is the producing
/// adapter's [`HarnessCapabilities`], needed to enforce check-completeness.
/// Returns `Err(reason)` on the first violation (tests `.unwrap()` it).
///
/// Invariants:
/// 1. `schema_version` matches the linked [`HARNESS_CONTRACT_VERSION`].
/// 2. `resulting_commit.is_some()` **iff** `outcome` is [`ChunkOutcome::Committed`],
///    the two oids agree, and the committed oid is a well-formed git object id.
/// 3. `changed_files` is non-empty only for a committed outcome.
/// 4. A [`ChunkOutcome::Failed`] carries a non-empty `reason`.
/// 5. Every `check_results` entry has a `check_id` matching a requested check,
///    with no duplicates. When the adapter `runs_checks` and execution reached
///    the check phase (any outcome but `Timeout`/`Cancelled`), the reported
///    check ids are exactly the requested set (completeness). When it does not
///    `runs_checks`, `check_results` is empty.
/// 6. A `transcript_ref`, when present, is a non-empty path.
pub fn assert_result_conforms(
    req: &ChunkRequest,
    res: &ChunkResult,
    caps: HarnessCapabilities,
) -> Result<(), String> {
    if res.schema_version != HARNESS_CONTRACT_VERSION {
        return Err(format!(
            "schema_version {} != linked contract version {HARNESS_CONTRACT_VERSION}",
            res.schema_version
        ));
    }

    match &res.outcome {
        ChunkOutcome::Committed { commit } => {
            if !is_oid(commit) {
                return Err(format!(
                    "Committed outcome carries a non-oid commit: {commit:?}"
                ));
            }
            match &res.resulting_commit {
                Some(rc) if rc == commit => {}
                Some(rc) => {
                    return Err(format!(
                        "resulting_commit ({rc}) disagrees with outcome commit ({commit})"
                    ));
                }
                None => return Err("Committed outcome but resulting_commit is None".into()),
            }
        }
        ChunkOutcome::Failed { reason } => {
            if reason.trim().is_empty() {
                return Err("Failed outcome carries an empty reason".into());
            }
        }
        ChunkOutcome::NoChange | ChunkOutcome::Timeout | ChunkOutcome::Cancelled => {}
    }

    if !matches!(res.outcome, ChunkOutcome::Committed { .. }) {
        if res.resulting_commit.is_some() {
            return Err(format!(
                "non-committed outcome {:?} must not carry resulting_commit",
                res.outcome
            ));
        }
        if !res.changed_files.is_empty() {
            return Err(format!(
                "non-committed outcome {:?} must not report changed_files",
                res.outcome
            ));
        }
    }

    // Check-result integrity: ids ⊆ requested, no duplicates.
    let requested: HashSet<&str> = req.checks.iter().map(|c| c.id.as_str()).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    for cr in &res.check_results {
        if !requested.contains(cr.check_id.as_str()) {
            return Err(format!(
                "check_results contains an un-requested check_id: {:?}",
                cr.check_id
            ));
        }
        if !seen.insert(cr.check_id.as_str()) {
            return Err(format!(
                "duplicate check_id in check_results: {:?}",
                cr.check_id
            ));
        }
    }
    if caps.runs_checks {
        // Timeout/Cancelled may stop before the check phase; every other outcome
        // must report a result for every requested check.
        if !matches!(res.outcome, ChunkOutcome::Timeout | ChunkOutcome::Cancelled)
            && seen != requested
        {
            return Err(format!(
                "runs_checks adapter reported {} check results for {} requested checks",
                seen.len(),
                requested.len()
            ));
        }
    } else if !res.check_results.is_empty() {
        return Err("adapter reports runs_checks=false but returned check_results".into());
    }

    if let Some(t) = &res.transcript_ref {
        if t.as_os_str().is_empty() {
            return Err("transcript_ref is present but empty".into());
        }
    }

    Ok(())
}

/// Run `harness.run_chunk(req)` and, on success, assert the result conforms to
/// the contract (using the harness's own capabilities) before returning it. The
/// single entry point an adapter's tests use so every returned result is
/// contract-gated automatically.
///
/// Test-only: it *panics* on non-conformance, which is the right behavior for a
/// test gate but would take down a supervisor if called in production — so it is
/// `#[cfg(test)]`. Production code that wants to validate an adapter result
/// should call [`assert_result_conforms`] and handle the `Err`.
///
/// Drives the adapter with a fresh, un-cancelled [`CancelToken`]. Use
/// [`run_and_check_with_cancel`] to exercise the cancellation path.
#[cfg(test)]
pub fn run_and_check(
    harness: &dyn super::CodeHarness,
    req: &ChunkRequest,
) -> Result<ChunkResult, super::HarnessError> {
    run_and_check_with_cancel(harness, req, &super::CancelToken::new())
}

/// [`run_and_check`] with a caller-supplied [`CancelToken`], so a test can trip
/// cancellation (before or during the run, from another thread) and still gate
/// the result through the structural contract.
#[cfg(test)]
pub fn run_and_check_with_cancel(
    harness: &dyn super::CodeHarness,
    req: &ChunkRequest,
    cancel: &super::CancelToken,
) -> Result<ChunkResult, super::HarnessError> {
    let out = harness.run_chunk(req, cancel);
    if let Ok(res) = &out {
        assert_result_conforms(req, res, harness.capabilities())
            .unwrap_or_else(|e| panic!("adapter produced a non-conforming ChunkResult: {e}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::stub::{StubBehavior, StubHarness};
    use super::super::{Check, ChunkOutcome, ChunkRequest, HarnessCapabilities, HarnessError};
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn req_with_checks(checks: Vec<Check>) -> ChunkRequest {
        ChunkRequest {
            run_id: "r".into(),
            chunk_id: "c".into(),
            attempt_id: "a".into(),
            worktree_path: PathBuf::from("/tmp/does-not-matter"),
            base_commit: "0".repeat(40),
            plan_rev: "v1".into(),
            brief: "b".into(),
            checks,
            files: vec![],
            timeout: None,
        }
    }

    fn one_check() -> Vec<Check> {
        vec![Check {
            id: "chk1".into(),
            desc: "d".into(),
            run: "true".into(),
            timeout: None,
        }]
    }

    /// Default (fully-capable) capabilities, matching `StubHarness::new`.
    fn full_caps() -> HarnessCapabilities {
        HarnessCapabilities {
            can_author_tests: true,
            reports_usage: true,
            honors_file_scope: true,
            runs_checks: true,
        }
    }

    // ---- The design §10 scenario matrix, run against the stub by default. ----

    #[test]
    fn scenario_clean_success() {
        let stub = StubHarness::new(StubBehavior::Commit {
            commit: "a".repeat(40),
            changed_files: vec![PathBuf::from("src/lib.rs")],
            fail_first_check: false,
        });
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(
            res.resulting_commit.as_deref(),
            Some("a".repeat(40).as_str())
        );
        assert_eq!(res.changed_files, vec![PathBuf::from("src/lib.rs")]);
        assert!(res.check_results.iter().all(|c| c.passed));
    }

    #[test]
    fn scenario_no_change() {
        let stub = StubHarness::new(StubBehavior::NoChange);
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::NoChange);
        assert!(res.resulting_commit.is_none());
        assert!(res.changed_files.is_empty());
    }

    #[test]
    fn scenario_self_check_failure() {
        let stub = StubHarness::new(StubBehavior::Commit {
            commit: "b".repeat(40),
            changed_files: vec![PathBuf::from("x")],
            fail_first_check: true,
        });
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        // Committed, but the self-check failed — the supervisor's floor decides
        // what to do; the harness just reports it.
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert!(!res.check_results[0].passed);
    }

    #[test]
    fn scenario_failed_run() {
        let stub = StubHarness::new(StubBehavior::Failed {
            reason: "provider 500".into(),
        });
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));
    }

    #[test]
    fn scenario_malformed_output_is_error() {
        let stub = StubHarness::new(StubBehavior::Error(HarnessError::MalformedOutput {
            message: "not json".into(),
        }));
        let req = req_with_checks(one_check());
        let err = run_and_check(&stub, &req).unwrap_err();
        assert!(matches!(err, HarnessError::MalformedOutput { .. }));
    }

    #[test]
    fn scenario_dirty_worktree_is_error() {
        let stub = StubHarness::new(StubBehavior::Error(HarnessError::DirtyWorktree {
            details: " M foo.rs".into(),
        }));
        let req = req_with_checks(one_check());
        let err = run_and_check(&stub, &req).unwrap_err();
        assert!(matches!(err, HarnessError::DirtyWorktree { .. }));
    }

    #[test]
    fn scenario_timeout_and_cancelled() {
        for behavior in [StubBehavior::Timeout, StubBehavior::Cancelled] {
            let stub = StubHarness::new(behavior.clone());
            let req = req_with_checks(one_check());
            let res = run_and_check(&stub, &req).unwrap();
            assert!(matches!(
                res.outcome,
                ChunkOutcome::Timeout | ChunkOutcome::Cancelled
            ));
        }
    }

    #[test]
    fn scenario_transcript_and_usage_capture() {
        let stub = StubHarness::new(StubBehavior::Commit {
            commit: "c".repeat(40),
            changed_files: vec![PathBuf::from("x")],
            fail_first_check: false,
        });
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        assert!(res.transcript_ref.is_some());
        let usage = res.usage.expect("stub reports usage by default");
        assert_eq!(usage.total_tokens, Some(150));
    }

    #[test]
    fn capabilities_can_suppress_usage_and_checks() {
        let stub = StubHarness::new(StubBehavior::Commit {
            commit: "d".repeat(40),
            changed_files: vec![],
            fail_first_check: false,
        })
        .with_capabilities(HarnessCapabilities {
            can_author_tests: false,
            reports_usage: false,
            honors_file_scope: false,
            runs_checks: false,
        });
        let req = req_with_checks(one_check());
        let res = run_and_check(&stub, &req).unwrap();
        assert!(res.usage.is_none());
        assert!(res.check_results.is_empty());
    }

    // ---- Invariant checker itself. ----

    #[test]
    fn conformance_rejects_commit_without_oid() {
        let req = req_with_checks(vec![]);
        let mut res = ChunkResult::committed("e".repeat(40), vec![]);
        res.resulting_commit = None; // corrupt it
        assert!(assert_result_conforms(&req, &res, full_caps()).is_err());
    }

    #[test]
    fn conformance_rejects_non_oid_commit() {
        let req = req_with_checks(vec![]);
        // A non-hex, wrong-length "commit" must be rejected.
        let res = ChunkResult::committed("not-a-real-oid", vec![]);
        assert!(assert_result_conforms(&req, &res, full_caps()).is_err());
    }

    #[test]
    fn conformance_rejects_wrong_schema_version() {
        let req = req_with_checks(vec![]);
        let mut res = ChunkResult::no_change();
        res.schema_version = HARNESS_CONTRACT_VERSION + 99;
        assert!(assert_result_conforms(&req, &res, full_caps()).is_err());
    }

    #[test]
    fn conformance_rejects_unrequested_check() {
        let req = req_with_checks(one_check());
        let mut res = ChunkResult::no_change();
        res.check_results.push(super::super::CheckResult {
            check_id: "not-requested".into(),
            desc: "sneaky".into(),
            run: "rm -rf /".into(),
            passed: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        });
        assert!(assert_result_conforms(&req, &res, full_caps()).is_err());
    }

    #[test]
    fn conformance_rejects_incomplete_checks_when_runs_checks() {
        // Two checks requested, adapter claims runs_checks, but reports none.
        let req = req_with_checks(vec![
            Check {
                id: "a".into(),
                desc: "d".into(),
                run: "true".into(),
                timeout: None,
            },
            Check {
                id: "b".into(),
                desc: "d".into(),
                run: "true".into(),
                timeout: None,
            },
        ]);
        let res = ChunkResult::no_change(); // no check_results
        assert!(assert_result_conforms(&req, &res, full_caps()).is_err());
    }

    #[test]
    fn conformance_accepts_stub_default() {
        let stub = StubHarness::new(StubBehavior::NoChange);
        let req = req_with_checks(one_check());
        // run_and_check panics on non-conformance; reaching here is the assert.
        run_and_check(&stub, &req).unwrap();
    }

    // ---- Cancellation + timeout, deterministically via the stub. ----

    #[test]
    fn cancel_pretripped_yields_cancelled_for_any_behavior() {
        use super::super::CancelToken;
        // A behavior that would otherwise commit; a pre-tripped token wins.
        let stub = StubHarness::new(StubBehavior::Commit {
            commit: "a".repeat(40),
            changed_files: vec![PathBuf::from("x")],
            fail_first_check: false,
        });
        let req = req_with_checks(one_check());
        let cancel = CancelToken::new();
        cancel.cancel();
        let res = run_and_check_with_cancel(&stub, &req, &cancel).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Cancelled);
        assert!(res.resulting_commit.is_none());
        assert!(res.check_results.is_empty());
    }

    #[test]
    fn slow_run_cancelled_in_flight() {
        use super::super::CancelToken;
        use std::time::Duration;
        // A generous budget so the run only ends via the in-flight cancel.
        let stub = StubHarness::new(StubBehavior::SlowUntilCancel {
            budget: Duration::from_secs(30),
        });
        let req = req_with_checks(one_check());
        let cancel = CancelToken::new();
        let trip = cancel.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            trip.cancel();
        });
        let res = run_and_check_with_cancel(&stub, &req, &cancel).unwrap();
        handle.join().unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Cancelled);
    }

    #[test]
    fn slow_run_times_out_when_not_cancelled() {
        use super::super::CancelToken;
        use std::time::Duration;
        // A zero budget expires immediately; the token is never tripped.
        let stub = StubHarness::new(StubBehavior::SlowUntilCancel {
            budget: Duration::from_millis(0),
        });
        let req = req_with_checks(one_check());
        let res = run_and_check_with_cancel(&stub, &req, &CancelToken::new()).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Timeout);
    }

    // ---- Live agent smoke tests (opt-in). ----
    //
    // These drive the REAL agents (network + credentials + a real binary), so
    // they are skipped unless `OCTL_HARNESS_LIVE=1`. Each also self-skips if its
    // binary is not installed, so enabling the gate on a partial toolbox never
    // fails spuriously. They assert only the *contract* (a conforming
    // `ChunkResult`, gated by `run_and_check`) — never a specific model output,
    // which is non-deterministic.

    /// Whether the live-agent gate (`OCTL_HARNESS_LIVE=1`) is enabled.
    fn live_enabled() -> bool {
        std::env::var("OCTL_HARNESS_LIVE").as_deref() == Ok("1")
    }

    /// Whether a bare binary name resolves on `PATH` (skip a live test whose tool
    /// is not installed).
    fn on_path(bin: &str) -> bool {
        std::env::var("PATH")
            .is_ok_and(|path| std::env::split_paths(&path).any(|d| d.join(bin).is_file()))
    }

    /// A real one-commit git repo in a temp dir, returned with its HEAD oid so a
    /// live `ChunkRequest` forks from genuine state.
    fn live_repo() -> (tempfile::TempDir, String) {
        use std::process::Command;
        let dir = tempfile::TempDir::new().unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "live@t"]);
        git(&["config", "user.name", "live"]);
        std::fs::write(dir.path().join("seed.txt"), "seed\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "seed"]);
        let head = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        (dir, head)
    }

    /// A trivial "create a file and commit it" live request against `repo`.
    fn live_request(repo: &std::path::Path, head: &str) -> ChunkRequest {
        ChunkRequest {
            run_id: "live".into(),
            chunk_id: "c1".into(),
            attempt_id: "a1".into(),
            worktree_path: repo.to_path_buf(),
            base_commit: head.into(),
            plan_rev: "v1".into(),
            brief: "Create a file named GREETING.txt whose only content is the word \
                    `hello`. Then commit it."
                .into(),
            checks: vec![Check {
                id: "exists".into(),
                desc: "GREETING.txt exists".into(),
                run: "test -f GREETING.txt".into(),
                timeout: Some(Duration::from_secs(10)),
            }],
            files: vec![PathBuf::from("GREETING.txt")],
            timeout: Some(Duration::from_secs(600)),
        }
    }

    #[test]
    fn live_claude_deepseek_conforms() {
        if !live_enabled() || !on_path("claude-deepseek") {
            return;
        }
        use super::super::claude::ClaudeHarness;
        let (repo, head) = live_repo();
        let h = ClaudeHarness::deepseek("flash");
        // `run_and_check` gates the result through the structural contract.
        let res = run_and_check(&h, &live_request(repo.path(), &head)).unwrap();
        // A live run may commit or (rarely) no-change; either is contract-valid.
        assert!(matches!(
            res.outcome,
            ChunkOutcome::Committed { .. } | ChunkOutcome::NoChange
        ));
    }

    #[test]
    fn live_pi_conforms() {
        if !live_enabled() || !on_path("pi") || std::env::var("DEEPSEEK_API_KEY").is_err() {
            return;
        }
        use super::super::pi::{PiConfig, PiHarness};
        let (repo, head) = live_repo();
        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let res = run_and_check(&h, &live_request(repo.path(), &head)).unwrap();
        assert!(matches!(
            res.outcome,
            ChunkOutcome::Committed { .. } | ChunkOutcome::NoChange
        ));
    }
}
