//! The deterministic correctness **floor** (design.md §4 — the panel's #1
//! non-negotiable) as a standalone, fully-tested module of pure gates plus a
//! thin impure capture layer.
//!
//! LLM verify is *advisory on top of a mechanical floor*, never the gate
//! itself (design §0.1, §4). The floor is the ground-truth oracle an
//! autonomous, unmonitored loop can trust: deterministic set/inequality rules,
//! **no LLM call and no judgment**. It enforces, against a **baseline snapshot**
//! captured at the `feat/<slug>` fork, that a merge is blocked unless:
//!
//! 1. the relevant `checks` pass;
//! 2. no test that passed at baseline is now failing (regression);
//! 3. no new clippy warnings vs baseline;
//! 4. the test suite was not gamed (count didn't drop, none newly
//!    `#[ignore]`d, none renamed-to-no-op/removed, assertion density held);
//! 5. the enumerated test-target set did not shrink vs baseline (a
//!    narrowed/empty test-binary set fails closed, not vacuously green);
//! 6. changed files stay within the chunk's declared `files_touched[]` scope
//!    (+ a configurable slack).
//!
//! # Layout
//!
//! - [`snapshot`] — the pure value model ([`BaselineSnapshot`], [`RunSnapshot`],
//!   [`TestSnapshot`], [`ClippySnapshot`], [`CheckRun`]) + `sha256` hashing that
//!   projects down to the `plan.json` [`octl_core::plan::Baseline`] shape.
//! - [`parse`] — pure parsers (cargo `--message-format=json`, libtest text +
//!   announced-vs-parsed reconcile, `assert*!` counting), exhaustively
//!   fixture-tested with no I/O.
//! - [`gates`] — the five pure gate functions + [`gates::evaluate_floor`] and
//!   the structured [`FloorVerdict`] / [`Violation`].
//! - [`runner`] — the impure capture layer (run checks/tests/clippy, count
//!   assertions on disk / at a git ref).
//! - [`git`] — the minimal git shell-outs capture needs (`diff --name-only`,
//!   `show <ref>:<path>`).
//!
//! # Purity boundary
//!
//! Gates are pure functions of already-collected snapshots/diffs — no LLM, no
//! I/O, deterministic in their inputs. Capture ([`runner`]/[`git`]) is the only
//! impure part and is deliberately thin, so the gate logic is all unit-testable
//! from fixtures (including adversarial ones — comment/string assertion padding,
//! line-shifted clippy warnings, was-failing-now-ignored tests).
//!
//! # Trust model — what is enforced, what remains
//!
//! `floor-capture-trust-model` closed the central injection surface: the floor
//! no longer trusts uncontrolled process *text*. **Enforced now:**
//!
//! - **Structured capture.** Clippy is read from cargo `--message-format=json`
//!   records keyed by lint code; tests are enumerated as `compiler-artifact`
//!   executables and run one binary at a time. A `println!`/`build.rs` cannot
//!   fabricate a clippy JSON record (a code-less build-script warning is
//!   dropped), and an injected `test x ... ok` line is caught below.
//! - **Fail-closed.** Every capture proves complete compilation + execution:
//!   unparseable cargo JSON, a missing `build-finished`, an `error`-level
//!   diagnostic, a **non-zero process exit despite a `build-finished` record**
//!   (a JSON-injection-then-SIGKILL), a libtest binary whose parsed counts
//!   disagree with its own announced summary, a **non-zero `filtered out`**
//!   count (a leaked test filter → subset capture), or an exit code inconsistent
//!   with that summary all reject with a [`FloorError`] — a compile failure or
//!   bad flag can never yield an empty snapshot that passes gates vacuously.
//! - **Target-qualified identity.** [`snapshot::TestId`] is
//!   `(package, target_kind, target, name)`, so a deleted test cannot be
//!   replaced by a same-named no-op in another target.
//! - **Execution isolation.** Capture subprocesses run under `env_clear()` + a
//!   small allow-list, so an inherited `RUSTFLAGS`/`RUSTDOCFLAGS`/`RUSTC_WRAPPER`
//!   cannot change the observed set.
//! - **Provenance-bound baseline.** [`snapshot::BaselineSnapshot`] pins the ref
//!   to an immutable commit OID (not a mutable ref string), records the
//!   `rustc -V` fingerprint + schema version, and exposes
//!   [`snapshot::BaselineSnapshot::verify_plan_baseline`] so a spec-node's plan
//!   baseline must match the live one. Assertion-count maps are read at a
//!   resolved OID ([`runner::assertion_counts_at_ref`]).
//! - **Floor-pinned target dir (round 2, F4).** Every cargo capture runs with a
//!   fresh, per-snapshot `CARGO_TARGET_DIR` (set on the process env — which beats
//!   an in-repo `build.target-dir` — plus an explicit `--target-dir` flag). The
//!   pipeline allocates a distinct dir for the fork baseline, each chunk tip, and
//!   the feature tip, so baseline and tip **never share a warm cache**: the
//!   bypass where `cargo clippy` on a cache the baseline warmed re-emits zero
//!   warnings and `gate_no_new_clippy` passes vacuously is closed
//!   ([`runner::capture_test_snapshot`] / the pipeline's `capture_snapshot`).
//! - **Enumeration integrity (round 2, F7).** Each capture records the enumerated
//!   `(package, target_kind, target)` test-target set
//!   ([`snapshot::TestSnapshot::targets`]); [`gate_enumeration_superset`] fails
//!   closed unless the tip's set is a **superset** of the baseline's. A narrowed
//!   or empty test-binary set (a workspace-narrowing alias, `--exclude`,
//!   `harness = false`, or a build that produced fewer harnesses) can no longer
//!   capture a smaller snapshot that passes vacuously — the vanished harness's
//!   tests would otherwise be invisible to every other gate.
//! - **Cross-component provenance (round 2, item 5 / F10).** The plan baseline
//!   carries `commit_oid` + `toolchain` + the enumerated-targets hash alongside
//!   the two content hashes, and [`snapshot::BaselineSnapshot::verify_plan_baseline`]
//!   requires **all** of them to equal one live-snapshot projection — so a plan
//!   cannot mix components captured at different commits/toolchains/enumerations.
//!   (Wired into the evaluator at T5; the pure check + projection land here.)
//!
//! **Remaining (deferred to a follow-up spin-off), still not tamper-proof:**
//!
//! - **Residual repo-controlled config.** The `build.target-dir` vector is closed
//!   (above), but cargo still honours other in-repo config: an `[alias]` that
//!   redirects the external `clippy` subcommand to a benign zero-warning command
//!   (the `test` subcommand is built-in and cannot be aliased), config
//!   `rustflags` / lint-level flips, and a consistently-weakening
//!   `rust-toolchain.toml` (the recorded toolchain catches baseline-vs-tip drift,
//!   not an evil-but-consistent pin). Full neutralization wants a `--config`-lock
//!   or out-of-tree invocation.
//! - **Custom test harness (`harness = false`).** A hand-written `main()` on a
//!   *test-producing* target can print perfectly balanced forged libtest output;
//!   the announced-vs-parsed reconcile cannot distinguish it from real libtest on
//!   stable. (A `harness = false` **bench** is legitimate and common — criterion
//!   — so a blanket reject is wrong; the fix must distinguish target kinds.)
//!   Deferred to the spin-off.
//! - **Doctests** (run by rustdoc, not a `compiler-artifact` binary) are not
//!   captured/target-qualified — new failing doctests, or a test moved *into* a
//!   doctest, are invisible. Capture is symmetric across baseline/tip so this is
//!   a gaming hole, not a crash; capturing them needs a separate `--doc` pass (or
//!   the nightly libtest JSON format). Deferred to the spin-off.
//! - Assertion density is still a **per-file, crude `assert*!` count**, not a
//!   semantic per-`#[test]` (AST) measure; `assert!(true)` padding is not
//!   detected.
//! - File-scope is a lexical allow-list with no `forbidden-even-if-declared`
//!   policy for validation-control files (`.cargo/config*`,
//!   `rust-toolchain.toml`, build scripts, CI).
//! - Wall-clock timeouts / output caps / process-group termination for capture
//!   subprocesses belong with the §9 circuit-breakers and are tracked there.
//!
//! # Wiring status
//!
//! The floor is consumed by the experimental code pipeline
//! (`crate::pipeline::live`), which runs [`evaluate_floor`] at each chunk /
//! feature merge boundary. It touches **no** event-append, reducer, or lock
//! path (the five state-integrity invariants): it only reads git and runs
//! commands, and writes nothing to the run projection set. T5 wires it into the
//! supervisor's own merge gate.

pub mod gates;
pub mod git;
pub mod parse;
pub mod runner;
pub mod snapshot;

pub use gates::{
    evaluate_floor, gate_checks_pass, gate_enumeration_superset, gate_file_scope,
    gate_no_new_clippy, gate_no_regression, gate_no_test_gaming, FloorInputs, FloorVerdict,
    GateKind, GateOutcome, Violation,
};
pub use snapshot::{
    hash_sorted, BaselineMismatch, BaselineSnapshot, CheckRun, ClippySnapshot, ClippyWarning,
    Coverage, RunSnapshot, TestId, TestSnapshot, BASELINE_SCHEMA_VERSION,
};

/// A failure in the floor's **impure capture layer** — running a command, or a
/// git shell-out. The pure gates never produce one; they return a
/// [`FloorVerdict`] regardless of pass/fail. Distinct from a gate *violation*
/// (a real regression/warning/out-of-scope file): a `FloorError` means the
/// floor could not *collect* what it needs to judge, and the caller must not
/// treat an incomplete capture as a passing floor.
#[derive(Debug, thiserror::Error)]
pub enum FloorError {
    /// A git shell-out failed (bad ref, not a repo, spawn error).
    #[error("floor git error: {message}")]
    Git {
        /// Diagnostic detail.
        message: String,
    },
    /// A capture command (tests/clippy) could not be run.
    #[error("floor capture error ({what}): {message}")]
    Capture {
        /// What was being captured (`"tests"`, `"clippy"`).
        what: &'static str,
        /// Diagnostic detail.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    //! End-to-end: capture from a real temp git repo, then run the full floor.
    //! Ties the impure capture layer to the pure gates on real inputs.

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::TempDir;

    use std::collections::BTreeSet;

    use super::runner::{assertion_counts_at_ref, assertion_counts_on_disk};
    use super::{
        evaluate_floor, git::changed_files, CheckRun, FloorInputs, RunSnapshot, TestId,
        TestSnapshot,
    };

    /// The single baseline test `t`, target-qualified.
    fn passed_t() -> BTreeSet<TestId> {
        [TestId::new("pkg", "lib", "pkg", "t")]
            .into_iter()
            .collect()
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
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
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn head(dir: &Path) -> String {
        String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string()
    }

    fn clean_check(passed: bool) -> CheckRun {
        CheckRun {
            desc: "feature check".into(),
            run: "cargo test".into(),
            cwd: None,
            passed,
            exit_code: Some(i32::from(!passed)),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// A repo with `src/a.rs` (2 assertions) committed as the baseline fork.
    fn repo_with_baseline() -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git_in(p, &["init", "-q", "-b", "main"]);
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(
            p.join("src/a.rs"),
            "#[test] fn t() { assert!(x); assert_eq!(a, b); }\n",
        )
        .unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "baseline fork"]);
        let base = head(p);
        (dir, base)
    }

    #[test]
    fn end_to_end_clean_change_passes_the_floor() {
        let (dir, base) = repo_with_baseline();
        let p = dir.path();
        let declared = vec![PathBuf::from("src/a.rs")];

        // Baseline assertion counts, from the fork ref.
        let base_assert = assertion_counts_at_ref(p, &base, &declared).unwrap();
        assert_eq!(base_assert[&PathBuf::from("src/a.rs")], 2);

        // An in-scope edit that ADDS an assertion (density up, not down).
        fs::write(
            p.join("src/a.rs"),
            "#[test] fn t() { assert!(x); assert_eq!(a, b); assert_ne!(c, d); }\n",
        )
        .unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "in-scope edit"]);

        let changed = changed_files(p, &base, "HEAD").unwrap();
        assert_eq!(changed, vec![PathBuf::from("src/a.rs")]);
        let cur_assert = assertion_counts_on_disk(p, &declared);

        let baseline = RunSnapshot {
            tests: TestSnapshot {
                passed: passed_t(),
                ..Default::default()
            },
            ..Default::default()
        };
        let current = baseline.clone();

        let inputs = FloorInputs {
            baseline: &baseline,
            current: &current,
            check_results: &[clean_check(true)],
            declared_files: &declared,
            changed_files: &changed,
            baseline_assertions: &base_assert,
            current_assertions: &cur_assert,
            file_scope_slack: 0,
        };
        let verdict = evaluate_floor(&inputs);
        assert!(verdict.passed(), "{verdict:#?}");
    }

    #[test]
    fn end_to_end_out_of_scope_and_gutted_assertions_fail() {
        let (dir, base) = repo_with_baseline();
        let p = dir.path();
        let declared = vec![PathBuf::from("src/a.rs")];
        let base_assert = assertion_counts_at_ref(p, &base, &declared).unwrap();

        // Gut the assertions in the declared file AND touch an out-of-scope one.
        fs::write(p.join("src/a.rs"), "#[test] fn t() {}\n").unwrap();
        fs::write(p.join("src/sneaky.rs"), "fn s() {}\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "gamed"]);

        let changed = changed_files(p, &base, "HEAD").unwrap();
        let cur_assert = assertion_counts_on_disk(p, &declared);
        assert_eq!(cur_assert[&PathBuf::from("src/a.rs")], 0); // gutted

        let snap = RunSnapshot {
            tests: TestSnapshot {
                passed: passed_t(),
                ..Default::default()
            },
            ..Default::default()
        };

        let inputs = FloorInputs {
            baseline: &snap,
            current: &snap,
            check_results: &[clean_check(true)],
            declared_files: &declared,
            changed_files: &changed,
            baseline_assertions: &base_assert,
            current_assertions: &cur_assert,
            file_scope_slack: 0,
        };
        let verdict = evaluate_floor(&inputs);
        assert!(!verdict.passed());

        // file-scope caught src/sneaky.rs; test-gaming caught the density drop.
        use super::{GateKind, Violation};
        assert!(verdict
            .failed_gates()
            .any(|g| g.gate == GateKind::FileScope));
        assert!(verdict.violations().any(|v| matches!(
            v,
            Violation::AssertionDensityRegressed { file, .. } if file == &PathBuf::from("src/a.rs")
        )));
    }

    #[test]
    fn floor_error_is_returned_when_capture_cannot_run() {
        // A bad baseline ref is a FloorError, not a silently-empty count map —
        // the caller must not read an incomplete capture as a green floor.
        let (dir, _base) = repo_with_baseline();
        let err =
            assertion_counts_at_ref(dir.path(), "deadbeefdeadbeef", &[PathBuf::from("src/a.rs")]);
        assert!(err.is_err());
    }
}
