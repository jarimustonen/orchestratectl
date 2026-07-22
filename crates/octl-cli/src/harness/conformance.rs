//! Reusable conformance suite for [`CodeHarness`] adapters (design.md §10: "A
//! conformance suite tests each adapter against: clean success, no-change,
//! self-check fail, malformed output, dirty worktree, transcript+usage capture,
//! …").
//!
//! The core is [`assert_result_conforms`] — the adapter-agnostic structural
//! invariants every [`ChunkResult`] must satisfy — and [`run_and_check`], which
//! runs an adapter and gates its result through those invariants. Any adapter
//! (the [`stub`], [`aider`], a future router) is "run through the suite" by
//! driving its scenarios and asserting via these helpers.
//!
//! The scenario matrix runs against the [`StubHarness`] by default (no network,
//! no git) so CI tests the *contract* deterministically. The live aider path is
//! an opt-in smoke test in [`aider`]'s own tests, gated on binaries/credentials
//! being present.
//!
//! [`stub`]: super::stub
//! [`aider`]: super::aider
//! [`StubHarness`]: super::stub::StubHarness

use super::{ChunkOutcome, ChunkRequest, ChunkResult, CodeHarness, HarnessError};

/// Assert a [`ChunkResult`] satisfies the structural contract every adapter must
/// honour, independent of which tool produced it. Returns `Err(reason)` on the
/// first violation so callers can surface it (tests `.unwrap()` it).
///
/// Invariants:
/// 1. `resulting_commit.is_some()` **iff** `outcome` is [`ChunkOutcome::Committed`],
///    and the two commit oids agree.
/// 2. A committed oid is non-empty.
/// 3. `changed_files` is non-empty only for a committed outcome.
/// 4. Every `check_results` entry corresponds to a requested check (matched by
///    its `run` command) — an adapter must not invent checks.
/// 5. A `transcript_ref`, when present, is a non-empty path.
pub fn assert_result_conforms(req: &ChunkRequest, res: &ChunkResult) -> Result<(), String> {
    match &res.outcome {
        ChunkOutcome::Committed { commit } => {
            if commit.is_empty() {
                return Err("Committed outcome carries an empty commit oid".into());
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
        ChunkOutcome::NoChange
        | ChunkOutcome::Failed { .. }
        | ChunkOutcome::Timeout
        | ChunkOutcome::Cancelled => {
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
    }

    let requested: Vec<&str> = req.checks.iter().map(|c| c.run.as_str()).collect();
    for cr in &res.check_results {
        if !requested.contains(&cr.run.as_str()) {
            return Err(format!(
                "check_results contains an un-requested check: {:?}",
                cr.run
            ));
        }
    }

    if let Some(t) = &res.transcript_ref {
        if t.as_os_str().is_empty() {
            return Err("transcript_ref is present but empty".into());
        }
    }

    Ok(())
}

/// Run `harness.run_chunk(req)` and, on success, assert the result conforms to
/// the contract before returning it. The single entry point an adapter's tests
/// use so every returned result is contract-gated automatically.
pub fn run_and_check(
    harness: &dyn CodeHarness,
    req: &ChunkRequest,
) -> Result<ChunkResult, HarnessError> {
    let out = harness.run_chunk(req);
    if let Ok(res) = &out {
        assert_result_conforms(req, res)
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
        }
    }

    fn one_check() -> Vec<Check> {
        vec![Check {
            desc: "d".into(),
            run: "true".into(),
        }]
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
        assert!(assert_result_conforms(&req, &res).is_err());
    }

    #[test]
    fn conformance_rejects_unrequested_check() {
        let req = req_with_checks(vec![Check {
            desc: "d".into(),
            run: "true".into(),
        }]);
        let mut res = ChunkResult::no_change();
        res.check_results.push(super::super::CheckResult {
            desc: "sneaky".into(),
            run: "rm -rf /".into(),
            passed: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        });
        assert!(assert_result_conforms(&req, &res).is_err());
    }

    #[test]
    fn conformance_accepts_stub_default() {
        let stub = StubHarness::new(StubBehavior::NoChange);
        let req = req_with_checks(one_check());
        // run_and_check panics on non-conformance; reaching here is the assert.
        run_and_check(&stub, &req).unwrap();
    }
}
