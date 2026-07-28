//! The deterministic-floor data model (design.md §4).
//!
//! Pure, serde-serializable value types the floor operates over: the per-run
//! [`RunSnapshot`] (test outcomes + clippy warnings + optional coverage), the
//! provenance-bound [`BaselineSnapshot`] captured at the `feat/<slug>` fork, and
//! the [`CheckRun`] result the check runner produces. Nothing here runs a
//! process or touches git — capture lives in [`super::runner`]/[`super::git`],
//! the gates in [`super::gates`]. Keeping the model separate makes every gate a
//! pure function of these values, unit-testable from fixtures with no I/O.
//!
//! Two identities in this module carry the injection-resistance work of
//! `floor-capture-trust-model`:
//!
//! - [`TestId`] is **target-qualified** — `(package, target_kind, target,
//!   name)`, not a bare libtest string. The same libtest path in a unit test
//!   and an integration binary are distinct ids, so a deleted test cannot be
//!   "replaced" by a same-named no-op in another target.
//! - [`ClippyWarning`] is built from cargo's `--message-format=json` records
//!   (lint code + package + primary-span file + message), not a parsed text
//!   line, so a `println!`/`build.rs` cannot fabricate one.

use std::collections::BTreeSet;
use std::fmt;

use octl_core::plan;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Schema version of the persisted [`BaselineSnapshot`] artifact. Bumped when
/// the on-disk shape changes so a stale artifact is rejected rather than
/// silently mis-read.
pub const BASELINE_SCHEMA_VERSION: u32 = 1;

/// Target-qualified identity of a single libtest test.
///
/// A bare libtest name (`export::csv::roundtrip`) is ambiguous across targets:
/// the same path can appear in a crate's unit tests, in an integration-test
/// binary, and in a doctest. Keying test identity on
/// `(package, target_kind, target, name)` — all sourced from cargo's structured
/// `--message-format=json` `compiler-artifact` records, never from parsed text —
/// makes a deleted test impossible to launder past
/// [`super::gates::gate_no_test_gaming`] by adding a same-named no-op in a
/// different target (`floor-capture-trust-model`).
///
/// Ordering is derived (field order: package, then kind, then target, then
/// name) so a `BTreeSet<TestId>` has a canonical, hash-stable iteration order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct TestId {
    /// Cargo package the target belongs to (e.g. `octl-cli`).
    pub package: String,
    /// Target kind as cargo reports it (`lib`, `bin`, `test`, `example`,
    /// `bench`). Distinguishes a unit test (`lib`) from an integration binary
    /// (`test`) of the same crate.
    pub target_kind: String,
    /// Target name (the crate name for `lib`, the file stem for a `test`
    /// binary).
    pub target: String,
    /// The libtest path within the target (e.g. `export::csv::roundtrip`).
    pub name: String,
}

impl TestId {
    /// Build a target-qualified id.
    pub fn new(
        package: impl Into<String>,
        target_kind: impl Into<String>,
        target: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            package: package.into(),
            target_kind: target_kind.into(),
            target: target.into(),
            name: name.into(),
        }
    }

    /// Canonical single-string form, used for hashing and human display:
    /// `package/target_kind/target::name`. Deterministic and reversible enough
    /// to diff by eye; the structured fields remain the source of truth.
    #[must_use]
    pub fn canonical(&self) -> String {
        format!(
            "{}/{}/{}::{}",
            self.package, self.target_kind, self.target, self.name
        )
    }
}

impl fmt::Display for TestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// The set of tests observed in one run, partitioned by outcome. Sets are
/// `BTreeSet<TestId>` so ordering is canonical (stable hashes, deterministic
/// diffs) and identity is target-qualified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSnapshot {
    /// Tests that passed.
    pub passed: BTreeSet<TestId>,
    /// Tests that failed.
    pub failed: BTreeSet<TestId>,
    /// Tests that were `#[ignore]`d / skipped.
    pub ignored: BTreeSet<TestId>,
    /// The enumerated test **targets** — one canonical `package/target_kind/target`
    /// per test-harness binary cargo built, independent of how many tests each
    /// runs (`floor-capture-hardening-round-2` item 2 / F7). A `harness = false`,
    /// `--exclude`, workspace-narrowing alias, or an empty build produces a
    /// *smaller* set than the fork's; the enumeration-integrity gate requires the
    /// tip's set to be a **superset** of the baseline's, so a narrowed/empty
    /// test-binary set fails closed instead of passing vacuously. Additive:
    /// `#[serde(default)]` so a pre-round-2 persisted snapshot (no targets) still
    /// deserializes.
    #[serde(default)]
    pub targets: BTreeSet<String>,
}

impl TestSnapshot {
    /// Every test observed, regardless of outcome. A well-formed run keeps the
    /// three sets disjoint; the union is defensive against overlap.
    #[must_use]
    pub fn all_ids(&self) -> BTreeSet<TestId> {
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

    /// Canonical string form of the passing set, for hashing / the
    /// `plan::Baseline` projection.
    #[must_use]
    pub fn passed_canonical(&self) -> BTreeSet<String> {
        self.passed.iter().map(TestId::canonical).collect()
    }
}

/// One clippy diagnostic, built from a cargo `--message-format=json`
/// `compiler-message` record (never from a parsed text line).
///
/// Identity is `(lint, package, file, message)`:
/// - `lint` is the structured lint code (`clippy::needless_return`,
///   `unused_variables`), the stable identity a message-wording change or a
///   line-shift does not perturb;
/// - `package`/`file` bind the observation to where it lives;
/// - `message` distinguishes two different instances of the same lint.
///
/// The `line:col` span is deliberately **excluded** so inserting a line above
/// an unchanged warning does not flip it to "new" (the T3 rationale, now
/// principled: the code is the identity, not the position). Two occurrences of
/// the same lint with the same message in the same file collapse to one
/// identity — the same narrow, documented trade-off, preferable to blocking a
/// line-shifting refactor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct ClippyWarning {
    /// The lint code (`clippy::needless_return`, `unused_variables`). A
    /// diagnostic with no code (e.g. a `build.rs` `cargo:warning=`) is dropped
    /// at capture, so this is always a real lint identity.
    pub lint: String,
    /// The package the diagnostic belongs to (short name).
    pub package: String,
    /// The primary span's file (repo-relative), or empty for a crate-level lint.
    pub file: String,
    /// The lint's short message (span line/col stripped).
    pub message: String,
}

impl ClippyWarning {
    /// Canonical single-string identity, used for hashing and human display.
    /// Fields are joined with a unit-separator byte so a field boundary can
    /// never be forged by embedding the delimiter in a value.
    #[must_use]
    pub fn canonical(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.lint, self.package, self.file, self.message
        )
    }
}

impl fmt::Display for ClippyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_empty() {
            write!(f, "[{}] {}: {}", self.package, self.lint, self.message)
        } else {
            write!(
                f,
                "[{}] {}:{}: {}",
                self.package, self.file, self.lint, self.message
            )
        }
    }
}

/// The set of clippy warnings observed in one run — structured
/// [`ClippyWarning`]s keyed by lint identity, so the "no new warnings" gate
/// compares lint codes, not text lines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClippySnapshot {
    /// Structured warning identities.
    pub warnings: BTreeSet<ClippyWarning>,
}

impl ClippySnapshot {
    /// Canonical string forms of the warnings, for hashing / the
    /// `plan::Baseline` projection.
    #[must_use]
    pub fn canonical(&self) -> BTreeSet<String> {
        self.warnings.iter().map(ClippyWarning::canonical).collect()
    }
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

/// A [`RunSnapshot`] bound to the commit it was captured at, plus the
/// provenance needed to prove that binding — the baseline the floor gates
/// enforce against (design.md §4: "captured at `feat/<slug>` fork").
///
/// # Provenance binding (`floor-capture-trust-model` item 5)
///
/// - `r#ref` is the human ref the snapshot was requested at (`feat/x@fork`),
///   kept for display only.
/// - `commit_oid` is that ref **resolved to an immutable commit OID at capture
///   time**. The gates and the assertion-count capture key off the OID, never
///   the mutable ref — a later force-push to the ref cannot silently change
///   what "baseline" means.
/// - `toolchain` fingerprints the `rustc -V` the snapshot was captured with, so
///   a baseline captured under a different toolchain than the tip is detectable.
/// - `schema_version` guards the persisted shape.
///
/// The per-file assertion-count maps are **not** carried here: the floor
/// measures assertion density against each chunk's *moving base commit* (the
/// current integration tip), not the fork, on purpose — so a later chunk that
/// guts a test an earlier chunk added is caught rather than hidden behind the
/// fork's lower count (see [`super::gates::gate_no_test_gaming`] and the
/// pipeline's `gate_chunk`). Binding a single fork-pinned assertion map here
/// would contradict that; [`super::runner::assertion_counts_at_ref`] instead
/// resolves and fails closed on its ref so each map is provably tied to a
/// commit. Folding the whole thing into one atomic `BaselineArtifact` (carrying
/// declared scope + command fingerprints) is deferred to the follow-up issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    /// Persisted-shape schema version ([`BASELINE_SCHEMA_VERSION`]).
    #[serde(default = "default_baseline_schema_version")]
    pub schema_version: u32,
    /// Git ref the snapshot was requested at (e.g. `feat/<slug>@fork`) — display
    /// only; `commit_oid` is authoritative.
    pub r#ref: String,
    /// The ref resolved to an immutable commit OID at capture time.
    pub commit_oid: String,
    /// `rustc -V` fingerprint the snapshot was captured with (provenance).
    #[serde(default)]
    pub toolchain: String,
    /// The observed state at that commit.
    pub snapshot: RunSnapshot,
}

fn default_baseline_schema_version() -> u32 {
    BASELINE_SCHEMA_VERSION
}

/// A mismatch between a live [`BaselineSnapshot`] and the `plan::Baseline` a
/// spec-node committed — returned by [`BaselineSnapshot::verify_plan_baseline`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineMismatch {
    /// The plan's baseline ref does not match the live snapshot's ref.
    Ref {
        /// Ref recorded in the plan.
        plan: String,
        /// Ref the live snapshot was captured at.
        live: String,
    },
    /// The plan's pinned commit OID does not match the live snapshot's — a
    /// baseline captured at a different commit than the one being gated
    /// (`floor-capture-hardening-round-2` item 5 / F10). Checked against the
    /// immutable OID, not the mutable `ref`, so a force-push cannot launder it.
    CommitOid {
        /// OID recorded in the plan.
        plan: String,
        /// OID the live snapshot was captured at.
        live: String,
    },
    /// The plan's toolchain fingerprint does not match the live snapshot's — a
    /// baseline captured under a different `rustc` than the one being gated.
    Toolchain {
        /// Toolchain recorded in the plan.
        plan: String,
        /// Toolchain the live snapshot was captured with.
        live: String,
    },
    /// The plan's test-pass-list hash disagrees with the live snapshot.
    TestPasslistHash,
    /// The plan's clippy-warning-list hash disagrees with the live snapshot.
    ClippyWarningsHash,
    /// The plan's enumerated-target-set hash disagrees with the live snapshot
    /// (F7): the plan was projected from a different enumeration than the one
    /// being gated.
    EnumeratedTargetsHash,
}

impl fmt::Display for BaselineMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaselineMismatch::Ref { plan, live } => {
                write!(f, "baseline ref mismatch: plan={plan:?} live={live:?}")
            }
            BaselineMismatch::CommitOid { plan, live } => {
                write!(
                    f,
                    "baseline commit-oid mismatch: plan={plan:?} live={live:?}"
                )
            }
            BaselineMismatch::Toolchain { plan, live } => {
                write!(
                    f,
                    "baseline toolchain mismatch: plan={plan:?} live={live:?}"
                )
            }
            BaselineMismatch::TestPasslistHash => {
                f.write_str("baseline test-passlist hash mismatch")
            }
            BaselineMismatch::ClippyWarningsHash => {
                f.write_str("baseline clippy-warnings hash mismatch")
            }
            BaselineMismatch::EnumeratedTargetsHash => {
                f.write_str("baseline enumerated-targets hash mismatch")
            }
        }
    }
}

impl std::error::Error for BaselineMismatch {}

impl BaselineSnapshot {
    /// Build a baseline from a ref, its resolved commit OID, a toolchain
    /// fingerprint, and the observed snapshot.
    pub fn new(
        r#ref: impl Into<String>,
        commit_oid: impl Into<String>,
        toolchain: impl Into<String>,
        snapshot: RunSnapshot,
    ) -> Self {
        Self {
            schema_version: BASELINE_SCHEMA_VERSION,
            r#ref: r#ref.into(),
            commit_oid: commit_oid.into(),
            toolchain: toolchain.into(),
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
            commit_oid: self.commit_oid.clone(),
            toolchain: self.toolchain.clone(),
            test_passlist_hash: hash_sorted(&self.snapshot.tests.passed_canonical()),
            clippy_warnings_hash: hash_sorted(&self.snapshot.clippy.canonical()),
            enumerated_targets_hash: hash_sorted(&self.snapshot.tests.targets),
            extra: serde_json::Map::new(),
        }
    }

    /// Require that a committed `plan::Baseline` describes *this* live snapshot:
    /// same ref, same test-pass-list hash, same clippy-warning-list hash. The
    /// evaluator (T5) calls this before trusting a plan so a spec-node cannot
    /// smuggle a baseline captured at a different commit / by a different
    /// algorithm than the one the supervisor gates against
    /// (`floor-capture-trust-model` item 5). Returns the first mismatch found.
    ///
    /// # Cross-component provenance (`floor-capture-hardening-round-2` item 5)
    ///
    /// Every component of the plan's baseline — the pinned `commit_oid`, the
    /// `toolchain`, and all three content hashes (test-passlist, clippy-warnings,
    /// enumerated-targets) — is required to equal the live snapshot's projection.
    /// Because the live side is a *single* [`to_plan_baseline`] of one
    /// [`RunSnapshot`], demanding equality on every field forces the plan's
    /// components to share one provenance: a plan that mixed a test hash from one
    /// capture with a clippy hash from another, or pinned a different OID /
    /// toolchain than the enumeration it hashed, cannot pass. The immutable
    /// `commit_oid` is checked in addition to the mutable `ref`, so a force-push
    /// to the ref cannot re-point what "baseline" means.
    ///
    /// [`to_plan_baseline`]: BaselineSnapshot::to_plan_baseline
    pub fn verify_plan_baseline(&self, plan: &plan::Baseline) -> Result<(), BaselineMismatch> {
        if plan.r#ref != self.r#ref {
            return Err(BaselineMismatch::Ref {
                plan: plan.r#ref.clone(),
                live: self.r#ref.clone(),
            });
        }
        if plan.commit_oid != self.commit_oid {
            return Err(BaselineMismatch::CommitOid {
                plan: plan.commit_oid.clone(),
                live: self.commit_oid.clone(),
            });
        }
        if plan.toolchain != self.toolchain {
            return Err(BaselineMismatch::Toolchain {
                plan: plan.toolchain.clone(),
                live: self.toolchain.clone(),
            });
        }
        let live = self.to_plan_baseline();
        if plan.test_passlist_hash != live.test_passlist_hash {
            return Err(BaselineMismatch::TestPasslistHash);
        }
        if plan.clippy_warnings_hash != live.clippy_warnings_hash {
            return Err(BaselineMismatch::ClippyWarningsHash);
        }
        if plan.enumerated_targets_hash != live.enumerated_targets_hash {
            return Err(BaselineMismatch::EnumeratedTargetsHash);
        }
        Ok(())
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
/// string, because [`plan::Check`] carries no id. The check's optional
/// `cwd`/`expect_exit` precision (locked in `plan-check-run-contract`) is
/// applied by the runner and folded into `passed`/`exit_code` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    /// Echoes [`plan::Check::desc`].
    pub desc: String,
    /// Echoes [`plan::Check::run`] (the command executed).
    pub run: String,
    /// Echoes [`plan::Check::cwd`] — the working directory the command ran in
    /// (relative to the run root), or `None` for the root. Kept so two checks
    /// that share a `run` but differ only in `cwd` produce distinguishable audit
    /// records. Skipped on the wire when absent, matching the plan shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Whether the command exited with the check's expected code
    /// ([`plan::Check::expect_exit`], default 0).
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

    fn tid(target_kind: &str, target: &str, name: &str) -> TestId {
        TestId::new("pkg", target_kind, target, name)
    }

    fn tset(ids: &[TestId]) -> BTreeSet<TestId> {
        ids.iter().cloned().collect()
    }

    #[test]
    fn total_counts_distinct_ids_across_partitions() {
        let ts = TestSnapshot {
            passed: tset(&[tid("lib", "pkg", "a"), tid("lib", "pkg", "b")]),
            failed: tset(&[tid("lib", "pkg", "c")]),
            ignored: tset(&[tid("lib", "pkg", "d")]),
            ..Default::default()
        };
        assert_eq!(ts.total(), 4);
        assert_eq!(ts.all_ids().len(), 4);
    }

    #[test]
    fn same_name_in_different_target_is_a_distinct_test() {
        // The whole point of target-qualification: `roundtrip` in the lib unit
        // tests and `roundtrip` in an integration binary are NOT the same id, so
        // deleting the real one cannot be masked by a same-named no-op elsewhere.
        let unit = tid("lib", "octl-cli", "export::roundtrip");
        let integ = tid("test", "e2e", "export::roundtrip");
        assert_ne!(unit, integ);
        let both = tset(&[unit.clone(), integ.clone()]);
        assert_eq!(both.len(), 2);
        assert_ne!(unit.canonical(), integ.canonical());
    }

    #[test]
    fn test_id_canonical_is_stable_and_display_matches() {
        let id = TestId::new("octl-cli", "lib", "octl-cli", "a::b");
        assert_eq!(id.canonical(), "octl-cli/lib/octl-cli::a::b");
        assert_eq!(id.to_string(), id.canonical());
    }

    #[test]
    fn clippy_warning_identity_ignores_line_but_keeps_lint_and_message() {
        let a = ClippyWarning {
            lint: "clippy::needless_return".into(),
            package: "pkg".into(),
            file: "src/a.rs".into(),
            message: "unneeded return".into(),
        };
        // Same lint+file+message ⇒ same identity regardless of where it sits.
        let b = a.clone();
        assert_eq!(a.canonical(), b.canonical());
        // A different lint code ⇒ different identity.
        let mut c = a.clone();
        c.lint = "clippy::redundant_clone".into();
        assert_ne!(a.canonical(), c.canonical());
    }

    #[test]
    fn hash_is_order_independent_and_content_sensitive() {
        let a = set(&["x", "y", "z"]);
        let b = set(&["z", "y", "x"]);
        assert_eq!(hash_sorted(&a), hash_sorted(&b));

        assert_ne!(
            hash_sorted(&set(&["a", "bc"])),
            hash_sorted(&set(&["ab", "c"]))
        );
        assert_ne!(hash_sorted(&set(&["a\nb"])), hash_sorted(&set(&["a", "b"])));
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
            "deadbeef",
            "rustc 1.97.1",
            RunSnapshot {
                tests: TestSnapshot {
                    passed: tset(&[tid("lib", "pkg", "t::a")]),
                    ..Default::default()
                },
                clippy: ClippySnapshot {
                    warnings: [ClippyWarning {
                        lint: "unused_variables".into(),
                        package: "pkg".into(),
                        file: "src/a.rs".into(),
                        message: "unused variable: `x`".into(),
                    }]
                    .into_iter()
                    .collect(),
                },
                coverage: None,
            },
        );
        let pb = base.to_plan_baseline();
        assert_eq!(pb.r#ref, "feat/x@fork");
        assert_eq!(
            pb.test_passlist_hash,
            hash_sorted(&set(&["pkg/lib/pkg::t::a"]))
        );
        assert!(pb.test_passlist_hash.starts_with("sha256:"));
        assert!(pb.clippy_warnings_hash.starts_with("sha256:"));
    }

    #[test]
    fn verify_plan_baseline_accepts_matching_and_rejects_drift() {
        let base = BaselineSnapshot::new(
            "feat/x@fork",
            "deadbeef",
            "rustc 1.97.1",
            RunSnapshot {
                tests: TestSnapshot {
                    passed: tset(&[tid("lib", "pkg", "t::a")]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // The plan projected from the same snapshot verifies.
        assert!(base.verify_plan_baseline(&base.to_plan_baseline()).is_ok());

        // A tampered test-pass-list hash is rejected.
        let mut bad = base.to_plan_baseline();
        bad.test_passlist_hash = "sha256:0".into();
        assert_eq!(
            base.verify_plan_baseline(&bad),
            Err(BaselineMismatch::TestPasslistHash)
        );

        // A different ref is rejected.
        let mut bad_ref = base.to_plan_baseline();
        bad_ref.r#ref = "feat/other@fork".into();
        assert!(matches!(
            base.verify_plan_baseline(&bad_ref),
            Err(BaselineMismatch::Ref { .. })
        ));

        // Provenance (`floor-capture-hardening-round-2` item 5): a baseline
        // captured at a different commit OID / toolchain is rejected even when
        // the two content hashes agree — the OID/toolchain are checked, not just
        // the mutable ref + hashes.
        let mut bad_oid = base.to_plan_baseline();
        bad_oid.commit_oid = "cafef00d".into();
        assert!(matches!(
            base.verify_plan_baseline(&bad_oid),
            Err(BaselineMismatch::CommitOid { .. })
        ));

        let mut bad_tc = base.to_plan_baseline();
        bad_tc.toolchain = "rustc 1.0.0".into();
        assert!(matches!(
            base.verify_plan_baseline(&bad_tc),
            Err(BaselineMismatch::Toolchain { .. })
        ));
    }

    #[test]
    fn verify_plan_baseline_rejects_enumerated_targets_drift() {
        // F7 provenance: the enumerated-target-set hash is a snapshot component
        // like the two content hashes — a plan projected from a different
        // enumeration (a narrowed target set) than the live one is rejected, so
        // the components must share one provenance.
        let base = BaselineSnapshot::new(
            "feat/x@fork",
            "deadbeef",
            "rustc 1.97.1",
            RunSnapshot {
                tests: TestSnapshot {
                    passed: tset(&[tid("lib", "pkg", "t::a")]),
                    targets: ["pkg/lib/pkg".to_string(), "pkg/test/e2e".to_string()]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // Its own projection verifies (all components share provenance).
        assert!(base.verify_plan_baseline(&base.to_plan_baseline()).is_ok());
        // A tampered enumerated-targets hash is rejected.
        let mut bad = base.to_plan_baseline();
        assert!(!bad.enumerated_targets_hash.is_empty());
        bad.enumerated_targets_hash = "sha256:0".into();
        assert_eq!(
            base.verify_plan_baseline(&bad),
            Err(BaselineMismatch::EnumeratedTargetsHash)
        );
    }

    #[test]
    fn to_plan_baseline_carries_provenance_fields() {
        let base = BaselineSnapshot::new(
            "feat/x@fork",
            "deadbeefoid",
            "rustc 1.97.1 (abc 2026-06-01)",
            RunSnapshot {
                tests: TestSnapshot {
                    targets: ["pkg/lib/pkg".to_string()].into_iter().collect(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let pb = base.to_plan_baseline();
        assert_eq!(pb.commit_oid, "deadbeefoid");
        assert_eq!(pb.toolchain, "rustc 1.97.1 (abc 2026-06-01)");
        assert_eq!(
            pb.enumerated_targets_hash,
            hash_sorted(&["pkg/lib/pkg".to_string()].into_iter().collect())
        );
        assert!(pb.enumerated_targets_hash.starts_with("sha256:"));
    }

    #[test]
    fn baseline_snapshot_round_trips_and_defaults_schema() {
        let base = BaselineSnapshot::new(
            "feat/x@fork",
            "deadbeef",
            "rustc 1.97.1",
            RunSnapshot::default(),
        );
        assert_eq!(base.schema_version, BASELINE_SCHEMA_VERSION);
        let json = serde_json::to_string(&base).unwrap();
        let back: BaselineSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(base, back);
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
                passed: tset(&[tid("lib", "pkg", "a")]),
                failed: tset(&[tid("lib", "pkg", "b")]),
                ignored: tset(&[tid("lib", "pkg", "c")]),
                targets: ["pkg/lib/pkg".to_string()].into_iter().collect(),
            },
            clippy: ClippySnapshot {
                warnings: [ClippyWarning {
                    lint: "unused_variables".into(),
                    package: "pkg".into(),
                    file: "src/a.rs".into(),
                    message: "w".into(),
                }]
                .into_iter()
                .collect(),
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
