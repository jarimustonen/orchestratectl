//! The deterministic-floor data model (design.md §4).
//!
//! Pure, serde-serializable value types the floor operates over: the per-run
//! [`RunSnapshot`] (test outcomes + clippy warnings + optional coverage), the
//! [`BaselineSnapshot`] captured at the `feat/<slug>` fork, and the
//! [`CheckRun`] result the check runner produces. Nothing here runs a process
//! or touches git — capture lives in [`super::runner`]/[`super::git`], the
//! gates in [`super::gates`]. Keeping the model separate makes every gate a
//! pure function of these values, unit-testable from fixtures with no I/O.

use std::collections::BTreeSet;

use octl_core::plan;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The set of tests observed in one run, partitioned by outcome. Sets are
/// `BTreeSet` so ordering is canonical (stable hashes, deterministic diffs).
/// A test id is the fully-qualified libtest name (e.g. `export::csv::roundtrip`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSnapshot {
    /// Test ids that passed.
    pub passed: BTreeSet<String>,
    /// Test ids that failed.
    pub failed: BTreeSet<String>,
    /// Test ids that were `#[ignore]`d / skipped.
    pub ignored: BTreeSet<String>,
}

impl TestSnapshot {
    /// Every test id observed, regardless of outcome. A well-formed run keeps
    /// the three sets disjoint; the union is defensive against overlap.
    #[must_use]
    pub fn all_ids(&self) -> BTreeSet<String> {
        self.passed
            .iter()
            .chain(&self.failed)
            .chain(&self.ignored)
            .cloned()
            .collect()
    }

    /// Total number of distinct tests observed (the test-count-gaming signal).
    #[must_use]
    pub fn total(&self) -> usize {
        self.all_ids().len()
    }
}

/// The set of clippy warnings observed in one run. Each entry is a normalized
/// warning line (short-format `path:line:col: warning: …`), so identical
/// warnings across runs compare equal for the "no new warnings" gate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClippySnapshot {
    /// Normalized warning identities.
    pub warnings: BTreeSet<String>,
}

/// Optional line-coverage figure captured alongside a snapshot (design.md §4
/// "optional coverage"). The floor records it for audit/trend; it is not a
/// hard gate here (coverage thresholds are orchestrator judgment, not the
/// mechanical floor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Lines executed at least once.
    pub covered_lines: u64,
    /// Total instrumented lines.
    pub total_lines: u64,
}

impl Coverage {
    /// Covered fraction in `[0.0, 1.0]`; `0.0` when nothing is instrumented.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total_lines == 0 {
            0.0
        } else {
            self.covered_lines as f64 / self.total_lines as f64
        }
    }
}

/// Everything the floor observes at one ref: test outcomes, clippy warnings,
/// and optional coverage. The baseline and the feature tip are both a
/// `RunSnapshot`; the gates diff one against the other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSnapshot {
    /// Test outcomes.
    pub tests: TestSnapshot,
    /// Clippy warnings.
    pub clippy: ClippySnapshot,
    /// Optional coverage figure.
    pub coverage: Option<Coverage>,
}

/// A [`RunSnapshot`] pinned to the git ref it was captured at — the baseline
/// the floor gates enforce against (design.md §4: "captured at `feat/<slug>`
/// fork"). Serde-serializable so it can be persisted to the run dir and
/// re-loaded on a later supervisor tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    /// Git ref the snapshot was taken at (e.g. `feat/<slug>@fork`).
    pub r#ref: String,
    /// The observed state at that ref.
    pub snapshot: RunSnapshot,
}

impl BaselineSnapshot {
    /// Build a baseline from a ref and its observed snapshot.
    pub fn new(r#ref: impl Into<String>, snapshot: RunSnapshot) -> Self {
        Self {
            r#ref: r#ref.into(),
            snapshot,
        }
    }

    /// Project this rich snapshot down to the hash-only [`plan::Baseline`] the
    /// `plan.json` contract carries (design.md §4/§7). The floor keeps the full
    /// lists (it needs them to diff); the plan records only their hashes, so a
    /// spec-node's `plan.json` and the supervisor's live baseline can be checked
    /// for agreement without embedding the whole list in the plan.
    #[must_use]
    pub fn to_plan_baseline(&self) -> plan::Baseline {
        plan::Baseline {
            r#ref: self.r#ref.clone(),
            test_passlist_hash: hash_sorted(&self.snapshot.tests.passed),
            clippy_warnings_hash: hash_sorted(&self.snapshot.clippy.warnings),
            extra: serde_json::Map::new(),
        }
    }
}

/// Deterministic `sha256:<hex>` of a sorted set of strings — the canonical
/// digest of a pass-list / warning-list. `BTreeSet` iteration is already
/// sorted, so the digest depends only on the contents, never on insertion
/// order.
///
/// Each element is **length-prefixed** (an 8-byte big-endian byte length
/// before the bytes) rather than delimiter-joined. A delimiter (`\n`) is
/// ambiguous when an element can itself contain that byte — a single
/// `"a\nb"` would hash identically to two elements `"a"` and `"b"`. libtest
/// ids can't contain a newline, but doc-test names and clippy messages can, and
/// this is a public function, so it frames unambiguously regardless of content.
#[must_use]
pub fn hash_sorted(items: &BTreeSet<String>) -> String {
    let mut h = Sha256::new();
    for item in items {
        h.update((item.len() as u64).to_be_bytes());
        h.update(item.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

/// The result of running one [`plan::Check`] — pass/fail plus captured output.
/// Produced by [`super::runner`]; consumed by the checks-pass gate. Mirrors the
/// harness [`crate::harness::CheckResult`] shape but keys on the check's `run`
/// string, because [`plan::Check`] carries no id (the richer `{cmd,cwd,
/// expect_exit}` check contract is the open decision `plan-check-run-contract`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    /// Echoes [`plan::Check::desc`].
    pub desc: String,
    /// Echoes [`plan::Check::run`] (the command executed).
    pub run: String,
    /// Whether the command exited 0.
    pub passed: bool,
    /// Process exit code, if the command ran to completion (`None` if it could
    /// not be spawned or was killed by a signal).
    pub exit_code: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn total_counts_distinct_ids_across_partitions() {
        let ts = TestSnapshot {
            passed: set(&["a", "b"]),
            failed: set(&["c"]),
            ignored: set(&["d"]),
        };
        assert_eq!(ts.total(), 4);
        assert_eq!(ts.all_ids(), set(&["a", "b", "c", "d"]));
    }

    #[test]
    fn hash_is_order_independent_and_content_sensitive() {
        // BTreeSet normalizes order, so two constructions hash the same.
        let a = set(&["x", "y", "z"]);
        let b = set(&["z", "y", "x"]);
        assert_eq!(hash_sorted(&a), hash_sorted(&b));

        // Length-prefix framing prevents concatenation collisions.
        assert_ne!(
            hash_sorted(&set(&["a", "bc"])),
            hash_sorted(&set(&["ab", "c"]))
        );

        // Framing is unambiguous even when an element contains a newline: a
        // single "a\nb" must not collide with two elements "a" and "b".
        assert_ne!(hash_sorted(&set(&["a\nb"])), hash_sorted(&set(&["a", "b"])));

        // A different member changes the digest.
        assert_ne!(hash_sorted(&a), hash_sorted(&set(&["x", "y"])));
        assert!(hash_sorted(&a).starts_with("sha256:"));
    }

    #[test]
    fn empty_set_hashes_stably() {
        let e1 = BTreeSet::new();
        let e2 = BTreeSet::new();
        assert_eq!(hash_sorted(&e1), hash_sorted(&e2));
    }

    #[test]
    fn to_plan_baseline_carries_ref_and_hashes() {
        let base = BaselineSnapshot::new(
            "feat/x@fork",
            RunSnapshot {
                tests: TestSnapshot {
                    passed: set(&["t::a"]),
                    ..Default::default()
                },
                clippy: ClippySnapshot {
                    warnings: set(&["src/a.rs:1:1: warning: w"]),
                },
                coverage: None,
            },
        );
        let pb = base.to_plan_baseline();
        assert_eq!(pb.r#ref, "feat/x@fork");
        assert_eq!(pb.test_passlist_hash, hash_sorted(&set(&["t::a"])));
        assert_eq!(
            pb.clippy_warnings_hash,
            hash_sorted(&set(&["src/a.rs:1:1: warning: w"]))
        );
        // A projected baseline round-trips through the plan validator's shape.
        assert!(pb.test_passlist_hash.starts_with("sha256:"));
    }

    #[test]
    fn coverage_fraction_handles_zero_total() {
        assert!(
            (Coverage {
                covered_lines: 0,
                total_lines: 0
            })
            .fraction()
            .abs()
                < f64::EPSILON
        );
        assert!(
            ((Coverage {
                covered_lines: 3,
                total_lines: 4
            })
            .fraction()
                - 0.75)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn snapshots_round_trip_through_serde() {
        let snap = RunSnapshot {
            tests: TestSnapshot {
                passed: set(&["a"]),
                failed: set(&["b"]),
                ignored: set(&["c"]),
            },
            clippy: ClippySnapshot {
                warnings: set(&["w1"]),
            },
            coverage: Some(Coverage {
                covered_lines: 1,
                total_lines: 2,
            }),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: RunSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }
}
