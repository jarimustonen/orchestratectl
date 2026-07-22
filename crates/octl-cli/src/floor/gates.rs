//! The deterministic floor gates (design.md §4) — pure functions returning a
//! structured verdict.
//!
//! Each `gate_*` function is a **pure** function of already-collected snapshots
//! and diffs: it makes no LLM call and no judgment, only mechanical
//! set/inequality checks. That is the whole point of the floor — a ground-truth
//! oracle the autonomous loop can trust *below* the advisory LLM verify layer
//! (design.md §0.1, §4). Capture (running tests/clippy/git) is impure and lives
//! in [`super::runner`]/[`super::git`]; the gates never do I/O.
//!
//! The gates enforced (design.md §4):
//! 1. [`gate_checks_pass`] — the relevant checks pass.
//! 2. [`gate_no_regression`] — no test that passed at baseline is now failing.
//! 3. [`gate_no_new_clippy`] — no new clippy warnings vs baseline.
//! 4. [`gate_no_test_gaming`] — test count didn't drop; none newly ignored;
//!    no baseline test vanished (rename-to-no-op); assertion density didn't
//!    regress in touched files.
//! 5. [`gate_file_scope`] — changed files stay within `files_touched[]` + slack.
//!
//! [`evaluate_floor`] runs all five over a [`FloorInputs`] and returns a
//! [`FloorVerdict`] the supervisor (at T5) will branch on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::snapshot::{CheckRun, ClippySnapshot, RunSnapshot, TestSnapshot};

/// Which floor gate a [`GateOutcome`] / [`Violation`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// The chunk/feature checks all pass.
    ChecksPass,
    /// No baseline-passing test now fails.
    NoRegression,
    /// No new clippy warning vs baseline.
    NoNewClippy,
    /// The test suite was not gamed (count/ignore/rename/assertion-density).
    NoTestGaming,
    /// Changed files stay within declared scope + slack.
    FileScope,
}

impl GateKind {
    /// Stable human label for logs/reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            GateKind::ChecksPass => "checks-pass",
            GateKind::NoRegression => "no-regression",
            GateKind::NoNewClippy => "no-new-clippy",
            GateKind::NoTestGaming => "no-test-gaming",
            GateKind::FileScope => "file-scope",
        }
    }
}

/// One specific, mechanical reason a gate failed. Serde-tagged so a supervisor
/// can record and route the exact violation (design.md §8 findings→action),
/// never a free-text blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Violation {
    /// A required check did not pass.
    CheckFailed {
        /// The check's description.
        desc: String,
        /// The command that failed.
        run: String,
        /// Its exit code, if it ran to completion.
        exit_code: Option<i32>,
    },
    /// A test that passed at baseline now fails.
    TestRegressed {
        /// The regressed test id.
        test: String,
    },
    /// A clippy warning present now but not at baseline.
    NewClippyWarning {
        /// The new warning's normalized identity line.
        warning: String,
    },
    /// The total test count dropped vs baseline.
    TestCountDropped {
        /// Distinct tests at baseline.
        baseline: usize,
        /// Distinct tests now.
        current: usize,
    },
    /// A test that passed at baseline is now `#[ignore]`d/skipped.
    NewlyIgnoredTest {
        /// The now-ignored test id.
        test: String,
    },
    /// A test present at baseline is entirely absent now (deleted or
    /// renamed-to-no-op) — the crude rename/removal signal.
    MissingBaselineTest {
        /// The vanished test id.
        test: String,
    },
    /// Assertion density in a touched file dropped vs baseline.
    AssertionDensityRegressed {
        /// The file whose assertion count dropped.
        file: PathBuf,
        /// `assert*!` occurrences at baseline.
        baseline: usize,
        /// `assert*!` occurrences now.
        current: usize,
    },
    /// A changed file lies outside the declared `files_touched[]` scope.
    OutOfScopeFile {
        /// The out-of-scope path.
        file: PathBuf,
    },
}

/// The outcome of one gate: pass/fail, a one-line summary, and the specific
/// violations when it failed (empty on pass).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOutcome {
    /// Which gate this is.
    pub gate: GateKind,
    /// Whether the gate passed.
    pub passed: bool,
    /// One-line human summary (the "reason", pass or fail).
    pub summary: String,
    /// Specific violations; empty iff `passed`.
    pub violations: Vec<Violation>,
}

impl GateOutcome {
    fn pass(gate: GateKind, summary: impl Into<String>) -> Self {
        Self {
            gate,
            passed: true,
            summary: summary.into(),
            violations: Vec::new(),
        }
    }

    fn fail(gate: GateKind, summary: impl Into<String>, violations: Vec<Violation>) -> Self {
        Self {
            gate,
            passed: false,
            summary: summary.into(),
            violations,
        }
    }
}

/// The aggregate verdict across every floor gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorVerdict {
    /// Per-gate outcomes, in [`GateKind`] declaration order.
    pub gates: Vec<GateOutcome>,
}

impl FloorVerdict {
    /// The floor passes iff **every** gate passes (design.md §4: a merge is
    /// blocked unless all hold).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.gates.iter().all(|g| g.passed)
    }

    /// Every violation across all failed gates.
    pub fn violations(&self) -> impl Iterator<Item = &Violation> {
        self.gates.iter().flat_map(|g| g.violations.iter())
    }

    /// The gates that failed.
    pub fn failed_gates(&self) -> impl Iterator<Item = &GateOutcome> {
        self.gates.iter().filter(|g| !g.passed)
    }
}

/// Everything [`evaluate_floor`] needs, all pre-collected by the impure capture
/// layer. Borrowed so the caller keeps ownership of the snapshots it persists.
#[derive(Debug, Clone, Copy)]
pub struct FloorInputs<'a> {
    /// Snapshot at the `feat/<slug>` fork.
    pub baseline: &'a RunSnapshot,
    /// Snapshot at the current feature/chunk tip.
    pub current: &'a RunSnapshot,
    /// Results of running the relevant `checks` (design.md §4 point 1).
    pub check_results: &'a [CheckRun],
    /// Declared `files_touched[]` for the chunk/feature.
    pub declared_files: &'a [PathBuf],
    /// Files actually changed (`git diff --name-only <base>..<tip>`).
    pub changed_files: &'a [PathBuf],
    /// `assert*!` counts per touched file at baseline.
    pub baseline_assertions: &'a BTreeMap<PathBuf, usize>,
    /// `assert*!` counts per touched file now.
    pub current_assertions: &'a BTreeMap<PathBuf, usize>,
    /// How many out-of-scope files to tolerate before failing file-scope.
    pub file_scope_slack: usize,
}

/// Run every floor gate over `inputs` and collect a [`FloorVerdict`]. Pure:
/// deterministic in its inputs, no I/O, no judgment.
#[must_use]
pub fn evaluate_floor(inputs: &FloorInputs) -> FloorVerdict {
    FloorVerdict {
        gates: vec![
            gate_checks_pass(inputs.check_results),
            gate_no_regression(&inputs.baseline.tests, &inputs.current.tests),
            gate_no_new_clippy(&inputs.baseline.clippy, &inputs.current.clippy),
            gate_no_test_gaming(
                &inputs.baseline.tests,
                &inputs.current.tests,
                inputs.baseline_assertions,
                inputs.current_assertions,
            ),
            gate_file_scope(
                inputs.declared_files,
                inputs.changed_files,
                inputs.file_scope_slack,
            ),
        ],
    }
}

/// Gate 1 — every relevant check passed (design.md §4 point 1).
#[must_use]
pub fn gate_checks_pass(results: &[CheckRun]) -> GateOutcome {
    let failed: Vec<Violation> = results
        .iter()
        .filter(|r| !r.passed)
        .map(|r| Violation::CheckFailed {
            desc: r.desc.clone(),
            run: r.run.clone(),
            exit_code: r.exit_code,
        })
        .collect();
    if failed.is_empty() {
        GateOutcome::pass(
            GateKind::ChecksPass,
            format!("all {} check(s) passed", results.len()),
        )
    } else {
        GateOutcome::fail(
            GateKind::ChecksPass,
            format!("{} of {} check(s) failed", failed.len(), results.len()),
            failed,
        )
    }
}

/// Gate 2 — no test that passed at baseline is now failing (design.md §4
/// point 2). A baseline-passing test that *vanished* is not a regression here
/// (it can't "fail" if it doesn't run); [`gate_no_test_gaming`] catches removal.
#[must_use]
pub fn gate_no_regression(baseline: &TestSnapshot, current: &TestSnapshot) -> GateOutcome {
    let regressed: Vec<Violation> = baseline
        .passed
        .intersection(&current.failed)
        .map(|t| Violation::TestRegressed { test: t.clone() })
        .collect();
    if regressed.is_empty() {
        GateOutcome::pass(GateKind::NoRegression, "no baseline-passing test now fails")
    } else {
        GateOutcome::fail(
            GateKind::NoRegression,
            format!("{} baseline-passing test(s) now fail", regressed.len()),
            regressed,
        )
    }
}

/// Gate 3 — no clippy warning present now that was absent at baseline
/// (design.md §4 point 3). Fixing a baseline warning is fine; introducing one
/// is not.
#[must_use]
pub fn gate_no_new_clippy(baseline: &ClippySnapshot, current: &ClippySnapshot) -> GateOutcome {
    let new: Vec<Violation> = current
        .warnings
        .difference(&baseline.warnings)
        .map(|w| Violation::NewClippyWarning { warning: w.clone() })
        .collect();
    if new.is_empty() {
        GateOutcome::pass(GateKind::NoNewClippy, "no new clippy warnings vs baseline")
    } else {
        GateOutcome::fail(
            GateKind::NoNewClippy,
            format!("{} new clippy warning(s) vs baseline", new.len()),
            new,
        )
    }
}

/// Gate 4 — the test suite was not gamed (design.md §4 point 4). Four crude,
/// mechanical signals, all reported together:
/// - **count dropped**: fewer distinct tests than baseline;
/// - **newly ignored**: a baseline-passing test is now `#[ignore]`d/skipped;
/// - **vanished**: a baseline test id is entirely absent now (delete / rename-to-no-op);
/// - **assertion density regressed**: a touched file has fewer `assert*!` than baseline.
#[must_use]
pub fn gate_no_test_gaming(
    baseline: &TestSnapshot,
    current: &TestSnapshot,
    baseline_assertions: &BTreeMap<PathBuf, usize>,
    current_assertions: &BTreeMap<PathBuf, usize>,
) -> GateOutcome {
    let mut violations = Vec::new();

    // Test count must not drop.
    let (base_total, cur_total) = (baseline.total(), current.total());
    if cur_total < base_total {
        violations.push(Violation::TestCountDropped {
            baseline: base_total,
            current: cur_total,
        });
    }

    // A baseline-passing test now ignored = gaming (a real skip hides a failure
    // or removes coverage). A brand-new test that happens to be ignored is not
    // flagged — only ones that used to pass.
    for test in baseline.passed.intersection(&current.ignored) {
        violations.push(Violation::NewlyIgnoredTest { test: test.clone() });
    }

    // A baseline test id entirely absent now — deleted or renamed to a no-op.
    let current_ids = current.all_ids();
    for test in &baseline.all_ids() {
        if !current_ids.contains(test) {
            violations.push(Violation::MissingBaselineTest { test: test.clone() });
        }
    }

    // Assertion density must not regress in a file present at both refs. A file
    // only in the current map (newly created) has no baseline to regress from.
    for (file, &base_count) in baseline_assertions {
        let cur_count = current_assertions.get(file).copied().unwrap_or(0);
        if cur_count < base_count {
            violations.push(Violation::AssertionDensityRegressed {
                file: file.clone(),
                baseline: base_count,
                current: cur_count,
            });
        }
    }

    if violations.is_empty() {
        GateOutcome::pass(
            GateKind::NoTestGaming,
            format!("no gaming signal ({cur_total} tests, assertion density held)"),
        )
    } else {
        GateOutcome::fail(
            GateKind::NoTestGaming,
            format!("{} test-gaming signal(s)", violations.len()),
            violations,
        )
    }
}

/// Gate 5 — file-scope (design.md §4: "File-scope is a merge-time
/// constraint"). Any changed file not in `declared` is out of scope; the gate
/// fails when the out-of-scope count exceeds `slack`. Out-of-scope files are
/// reported as violations only when the gate fails (within-slack drift is noted
/// in the summary, not raised as a violation).
///
/// Paths are compared verbatim (already-canonical repo-relative paths — the
/// plan validator's `is_safe_repo_relative` rejects `.`/`..`/`//` components at
/// authoring time, so both sides are normal forms). This is a lexical guard;
/// symlink resolution is out of scope (as `plan.rs` documents).
#[must_use]
pub fn gate_file_scope(declared: &[PathBuf], changed: &[PathBuf], slack: usize) -> GateOutcome {
    let declared_set: BTreeSet<&PathBuf> = declared.iter().collect();
    let out_of_scope: Vec<PathBuf> = changed
        .iter()
        .filter(|f| !declared_set.contains(f))
        .cloned()
        .collect();

    if out_of_scope.len() <= slack {
        GateOutcome::pass(
            GateKind::FileScope,
            format!(
                "{} changed file(s), {} out-of-scope within slack {}",
                changed.len(),
                out_of_scope.len(),
                slack
            ),
        )
    } else {
        let n = out_of_scope.len();
        GateOutcome::fail(
            GateKind::FileScope,
            format!("{n} out-of-scope file(s) exceed slack {slack}"),
            out_of_scope
                .into_iter()
                .map(|file| Violation::OutOfScopeFile { file })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tset(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(ToString::to_string).collect()
    }

    fn check(desc: &str, run: &str, passed: bool, exit: Option<i32>) -> CheckRun {
        CheckRun {
            desc: desc.to_string(),
            run: run.to_string(),
            passed,
            exit_code: exit,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    // --- gate 1: checks pass ---

    #[test]
    fn checks_pass_when_all_green() {
        let g = gate_checks_pass(&[
            check("a", "cargo test a", true, Some(0)),
            check("b", "cargo test b", true, Some(0)),
        ]);
        assert!(g.passed);
        assert!(g.violations.is_empty());
    }

    #[test]
    fn checks_fail_reports_each_failure() {
        let g = gate_checks_pass(&[
            check("a", "cargo test a", true, Some(0)),
            check("b", "cargo test b", false, Some(101)),
        ]);
        assert!(!g.passed);
        assert_eq!(g.violations.len(), 1);
        assert_eq!(
            g.violations[0],
            Violation::CheckFailed {
                desc: "b".into(),
                run: "cargo test b".into(),
                exit_code: Some(101),
            }
        );
    }

    #[test]
    fn empty_checks_pass_vacuously() {
        // No checks defined ⇒ nothing to fail. (The plan validator, not the
        // floor, enforces "≥1 check per chunk"; the gate reports what it's given.)
        assert!(gate_checks_pass(&[]).passed);
    }

    // --- gate 2: regression ---

    #[test]
    fn no_regression_on_clean_pass() {
        let base = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        assert!(gate_no_regression(&base, &cur).passed);
    }

    #[test]
    fn regression_when_baseline_pass_now_fails() {
        let base = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a"]),
            failed: tset(&["b"]),
            ..Default::default()
        };
        let g = gate_no_regression(&base, &cur);
        assert!(!g.passed);
        assert_eq!(
            g.violations,
            vec![Violation::TestRegressed { test: "b".into() }]
        );
    }

    #[test]
    fn new_test_failing_is_not_a_regression() {
        // `c` never passed at baseline, so its failure is not a *regression*.
        let base = TestSnapshot {
            passed: tset(&["a"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a"]),
            failed: tset(&["c"]),
            ..Default::default()
        };
        assert!(gate_no_regression(&base, &cur).passed);
    }

    // --- gate 3: clippy ---

    #[test]
    fn no_new_clippy_when_subset() {
        let base = ClippySnapshot {
            warnings: tset(&["w1", "w2"]),
        };
        let cur = ClippySnapshot {
            warnings: tset(&["w1"]), // fixed one, added none
        };
        assert!(gate_no_new_clippy(&base, &cur).passed);
    }

    #[test]
    fn new_clippy_warning_fails() {
        let base = ClippySnapshot {
            warnings: tset(&["w1"]),
        };
        let cur = ClippySnapshot {
            warnings: tset(&["w1", "w2"]),
        };
        let g = gate_no_new_clippy(&base, &cur);
        assert!(!g.passed);
        assert_eq!(
            g.violations,
            vec![Violation::NewClippyWarning {
                warning: "w2".into()
            }]
        );
    }

    // --- gate 4: test gaming ---

    #[test]
    fn no_gaming_on_clean_or_expanded_suite() {
        let base = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a", "b", "c"]), // added a test, dropped none
            ..Default::default()
        };
        let mut assertions = BTreeMap::new();
        assertions.insert(PathBuf::from("src/a.rs"), 3);
        let g = gate_no_test_gaming(&base, &cur, &assertions, &assertions);
        assert!(g.passed, "{:?}", g.violations);
    }

    #[test]
    fn detects_count_drop_and_vanished_test() {
        let base = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a"]), // b removed
            ..Default::default()
        };
        let g = gate_no_test_gaming(&base, &cur, &BTreeMap::new(), &BTreeMap::new());
        assert!(!g.passed);
        assert!(g.violations.contains(&Violation::TestCountDropped {
            baseline: 2,
            current: 1
        }));
        assert!(g
            .violations
            .contains(&Violation::MissingBaselineTest { test: "b".into() }));
    }

    #[test]
    fn detects_newly_ignored_baseline_pass() {
        let base = TestSnapshot {
            passed: tset(&["a", "b"]),
            ..Default::default()
        };
        let cur = TestSnapshot {
            passed: tset(&["a"]),
            ignored: tset(&["b"]), // b was passing, now ignored
            ..Default::default()
        };
        let g = gate_no_test_gaming(&base, &cur, &BTreeMap::new(), &BTreeMap::new());
        assert!(!g.passed);
        assert!(g
            .violations
            .contains(&Violation::NewlyIgnoredTest { test: "b".into() }));
        // Count did not drop (b still counted, just ignored), and b is still
        // present, so neither of those signals fire.
        assert!(!g
            .violations
            .iter()
            .any(|v| matches!(v, Violation::TestCountDropped { .. })));
        assert!(!g
            .violations
            .iter()
            .any(|v| matches!(v, Violation::MissingBaselineTest { .. })));
    }

    #[test]
    fn detects_assertion_density_regression() {
        let ts = TestSnapshot {
            passed: tset(&["a"]),
            ..Default::default()
        };
        let mut base = BTreeMap::new();
        base.insert(PathBuf::from("src/a.rs"), 5);
        let mut cur = BTreeMap::new();
        cur.insert(PathBuf::from("src/a.rs"), 2); // gutted assertions
        let g = gate_no_test_gaming(&ts, &ts, &base, &cur);
        assert!(!g.passed);
        assert!(g
            .violations
            .contains(&Violation::AssertionDensityRegressed {
                file: PathBuf::from("src/a.rs"),
                baseline: 5,
                current: 2,
            }));
    }

    #[test]
    fn added_assertions_and_new_files_do_not_regress() {
        let ts = TestSnapshot {
            passed: tset(&["a"]),
            ..Default::default()
        };
        let mut base = BTreeMap::new();
        base.insert(PathBuf::from("src/a.rs"), 2);
        let mut cur = BTreeMap::new();
        cur.insert(PathBuf::from("src/a.rs"), 4); // more assertions
        cur.insert(PathBuf::from("src/new.rs"), 1); // brand-new file
        assert!(gate_no_test_gaming(&ts, &ts, &base, &cur).passed);
    }

    // --- gate 5: file scope ---

    #[test]
    fn file_scope_passes_within_declared() {
        let declared = paths(&["src/a.rs", "src/a_test.rs"]);
        let changed = paths(&["src/a.rs"]);
        assert!(gate_file_scope(&declared, &changed, 0).passed);
    }

    #[test]
    fn file_scope_fails_out_of_scope_beyond_slack() {
        let declared = paths(&["src/a.rs"]);
        let changed = paths(&["src/a.rs", "src/secret.rs", "Cargo.toml"]);
        let g = gate_file_scope(&declared, &changed, 0);
        assert!(!g.passed);
        assert_eq!(g.violations.len(), 2);
        assert!(g.violations.contains(&Violation::OutOfScopeFile {
            file: PathBuf::from("src/secret.rs")
        }));
        assert!(g.violations.contains(&Violation::OutOfScopeFile {
            file: PathBuf::from("Cargo.toml")
        }));
    }

    #[test]
    fn file_scope_tolerates_within_slack() {
        let declared = paths(&["src/a.rs"]);
        let changed = paths(&["src/a.rs", "src/extra.rs"]);
        // One out-of-scope file, slack 1 ⇒ pass, no violations raised.
        let g = gate_file_scope(&declared, &changed, 1);
        assert!(g.passed);
        assert!(g.violations.is_empty());
    }

    // --- aggregate ---

    #[test]
    fn evaluate_floor_is_green_on_a_clean_run() {
        let base = RunSnapshot {
            tests: TestSnapshot {
                passed: tset(&["a"]),
                ..Default::default()
            },
            clippy: ClippySnapshot {
                warnings: tset(&["w1"]),
            },
            coverage: None,
        };
        let cur = base.clone();
        let declared = paths(&["src/a.rs"]);
        let changed = paths(&["src/a.rs"]);
        let assertions = BTreeMap::new();
        let inputs = FloorInputs {
            baseline: &base,
            current: &cur,
            check_results: &[check("c", "cargo test", true, Some(0))],
            declared_files: &declared,
            changed_files: &changed,
            baseline_assertions: &assertions,
            current_assertions: &assertions,
            file_scope_slack: 0,
        };
        let verdict = evaluate_floor(&inputs);
        assert!(verdict.passed(), "{verdict:#?}");
        assert_eq!(verdict.gates.len(), 5);
        assert_eq!(verdict.violations().count(), 0);
    }

    #[test]
    fn evaluate_floor_aggregates_multiple_gate_failures() {
        let base = RunSnapshot {
            tests: TestSnapshot {
                passed: tset(&["a", "b"]),
                ..Default::default()
            },
            clippy: ClippySnapshot::default(),
            coverage: None,
        };
        let cur = RunSnapshot {
            tests: TestSnapshot {
                passed: tset(&["a"]),
                failed: tset(&["b"]), // regression
                ..Default::default()
            },
            clippy: ClippySnapshot {
                warnings: tset(&["new-warn"]), // new clippy
            },
            coverage: None,
        };
        let declared = paths(&["src/a.rs"]);
        let changed = paths(&["src/a.rs", "src/out.rs"]); // out of scope
        let assertions = BTreeMap::new();
        let inputs = FloorInputs {
            baseline: &base,
            current: &cur,
            check_results: &[check("c", "cargo test", false, Some(101))], // check fail
            declared_files: &declared,
            changed_files: &changed,
            baseline_assertions: &assertions,
            current_assertions: &assertions,
            file_scope_slack: 0,
        };
        let verdict = evaluate_floor(&inputs);
        assert!(!verdict.passed());
        // checks, regression, clippy, file-scope all fail (4 gates).
        assert_eq!(verdict.failed_gates().count(), 4);
        // no-test-gaming still passes: b failing still counts + is present.
        assert!(
            verdict
                .gates
                .iter()
                .find(|g| g.gate == GateKind::NoTestGaming)
                .unwrap()
                .passed
        );
    }
}
