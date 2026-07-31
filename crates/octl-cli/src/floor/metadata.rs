//! Trusted workspace metadata for the floor (`floor-capture-hardening-round-3`
//! items 2–4).
//!
//! The floor's other captures observe what a repo-influenced `cargo` *chose* to
//! build. This module adds an **independent** source of the expected
//! `(package, target_kind, target)` universe by shelling `cargo metadata` (argv,
//! isolated) and parsing each package's `Cargo.toml`, so the capture can be
//! judged against ground truth rather than only relative to a (possibly
//! compromised or already-empty) baseline:
//!
//! - [`expected_test_targets`] derives the confident set of test-producing
//!   targets (lib + integration `test` kinds, minus any the manifest disables via
//!   `test = false` / `harness = false`). [`super::runner::capture_test_snapshot`]
//!   fails closed unless the captured enumeration is a **superset** of it, and on
//!   an **empty** enumeration when metadata says test targets exist (item 2). This
//!   is *absolute*, so a baseline whose enumeration is already empty/narrowed no
//!   longer passes vacuously — the gap the round-2 baseline-relative superset gate
//!   could not close.
//! - [`forged_harness_targets`] rejects an undeclared custom test harness
//!   (`harness = false` on a *test-producing* target — `[lib]` / `[[bin]]` /
//!   `[[test]]`), while allowing a legitimate `[[bench]] harness = false`
//!   (criterion-style benches; this repo has one). A hand-written `main()` on a
//!   test-producing target can print perfectly balanced forged libtest output the
//!   announced-vs-parsed reconcile cannot distinguish from real libtest, so the
//!   floor refuses to trust such a capture at all (item 3 / F5).
//! - [`WorkspaceMetadata::package_names`] gives the per-package list the doctest
//!   pass ([`super::runner::capture_doctests`]) iterates (item 4 / F6).
//!
//! Manifest parsing ([`parse_manifest_targets`]) and the two derivations are pure
//! and unit-tested from fixtures; only [`load`] does I/O.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::FloorError;

/// A workspace package from `cargo metadata` (`--no-deps`, so only local
/// members), reduced to the fields the floor needs.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageMeta {
    /// Package name (matches the short name capture records).
    pub name: String,
    /// Absolute path to the package's `Cargo.toml`.
    pub manifest_path: PathBuf,
    /// The package's build targets.
    #[serde(default)]
    pub targets: Vec<TargetMeta>,
}

/// A single build target from `cargo metadata`.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetMeta {
    /// Target name (crate name for a lib, file stem for a test/bin).
    pub name: String,
    /// Target kinds (`["lib"]`, `["test"]`, `["bin"]`, `["bench"]`, …).
    #[serde(default)]
    pub kind: Vec<String>,
    /// Cargo features this target requires to be built (`required-features` in the
    /// manifest). A default-feature `cargo test` does **not** build a target with
    /// unmet required-features, so such a target must be excluded from the expected
    /// set to avoid a false-block (`floor-capture-hardening-round-3` item 2, review
    /// follow-up).
    #[serde(default, rename = "required-features")]
    pub required_features: Vec<String>,
}

impl TargetMeta {
    /// The first (primary) kind, or `""` when cargo omitted it.
    #[must_use]
    fn primary_kind(&self) -> &str {
        self.kind.first().map_or("", String::as_str)
    }
}

/// The trusted workspace metadata: the local packages and their targets.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceMetadata {
    /// Local workspace packages (with `--no-deps`, dependencies are excluded).
    #[serde(default)]
    pub packages: Vec<PackageMeta>,
}

impl WorkspaceMetadata {
    /// Every local package name, sorted and de-duplicated — the per-package list
    /// the doctest pass iterates.
    #[must_use]
    pub fn package_names(&self) -> BTreeSet<String> {
        self.packages.iter().map(|p| p.name.clone()).collect()
    }
}

/// Load trusted workspace metadata by shelling `cargo metadata` (argv, isolated,
/// `--no-deps` so only local members are reported, `--format-version 1` for a
/// stable shape). Fails closed on any spawn/exit/parse error — the floor must not
/// silently treat "could not determine the expected universe" as "nothing is
/// expected".
pub fn load(cwd: &Path) -> Result<WorkspaceMetadata, FloorError> {
    let out = super::runner::isolated_command(&super::runner::cargo_bin(), None)
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "metadata",
            message: format!("could not run `cargo metadata`: {e}"),
        })?;
    if !out.status.success() {
        return Err(FloorError::Capture {
            what: "metadata",
            message: format!(
                "`cargo metadata` exited {:?}; failing closed. stderr: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|e| FloorError::Capture {
        what: "metadata",
        message: format!("could not parse `cargo metadata` output: {e}"),
    })
}

/// Canonical `package/target_kind/target` key — the same shape
/// [`super::runner::capture_test_snapshot`] records in
/// [`super::snapshot::TestSnapshot::targets`].
fn target_key(package: &str, kind: &str, target: &str) -> String {
    format!("{package}/{kind}/{target}")
}

// ---------------------------------------------------------------------------
// Cargo.toml target-table parsing (pure)
// ---------------------------------------------------------------------------

/// The manifest target-table kinds the floor distinguishes. A `[[bench]]` may
/// legitimately set `harness = false` (criterion); the test-producing kinds may
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    /// `[lib]`.
    Lib,
    /// `[[bin]]`.
    Bin,
    /// `[[test]]`.
    Test,
    /// `[[bench]]` — the one kind for which `harness = false` is legitimate.
    Bench,
    /// `[[example]]`.
    Example,
}

impl ManifestKind {
    /// True for kinds that produce a libtest harness under `cargo test` — the
    /// kinds on which an undeclared `harness = false` is a forge risk.
    #[must_use]
    fn is_test_producing(self) -> bool {
        matches!(
            self,
            ManifestKind::Lib | ManifestKind::Bin | ManifestKind::Test
        )
    }
}

/// A parsed manifest target table with the flags the floor reads. `name` is
/// absent for a `[lib]` (a package has exactly one, named after the crate);
/// present for the array-of-tables kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestTarget {
    /// Which target-table kind this is.
    pub kind: ManifestKind,
    /// Declared `name`, if any.
    pub name: Option<String>,
    /// `harness = <bool>`, if set (default is `true`).
    pub harness: Option<bool>,
    /// `test = <bool>`, if set (default is `true` for lib/test, false for bench).
    pub test: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawManifest {
    lib: Option<RawTarget>,
    #[serde(default)]
    bin: Vec<RawTarget>,
    #[serde(default)]
    test: Vec<RawTarget>,
    #[serde(default)]
    bench: Vec<RawTarget>,
    #[serde(default)]
    example: Vec<RawTarget>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTarget {
    name: Option<String>,
    harness: Option<bool>,
    test: Option<bool>,
}

impl RawTarget {
    fn into_manifest(self, kind: ManifestKind) -> ManifestTarget {
        ManifestTarget {
            kind,
            name: self.name,
            harness: self.harness,
            test: self.test,
        }
    }
}

/// Parse the `[lib]` / `[[bin]]` / `[[test]]` / `[[bench]]` / `[[example]]`
/// target tables out of a `Cargo.toml` string. Pure and total: a manifest that
/// does not parse as TOML, or that declares none of these tables, yields an empty
/// vector (the caller decides how to treat "no explicit tables" — auto-discovered
/// targets still come from `cargo metadata`).
#[must_use]
pub fn parse_manifest_targets(manifest: &str) -> Vec<ManifestTarget> {
    let Ok(raw) = toml::from_str::<RawManifest>(manifest) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(lib) = raw.lib {
        out.push(lib.into_manifest(ManifestKind::Lib));
    }
    for b in raw.bin {
        out.push(b.into_manifest(ManifestKind::Bin));
    }
    for t in raw.test {
        out.push(t.into_manifest(ManifestKind::Test));
    }
    for b in raw.bench {
        out.push(b.into_manifest(ManifestKind::Bench));
    }
    for e in raw.example {
        out.push(e.into_manifest(ManifestKind::Example));
    }
    out
}

/// The test-producing targets in one manifest that set `harness = false` — a
/// forged custom harness (item 3 / F5). A `[[bench]] harness = false` is
/// legitimate and never reported.
#[must_use]
pub fn forged_harness_in_manifest(manifest: &str) -> Vec<ManifestTarget> {
    parse_manifest_targets(manifest)
        .into_iter()
        .filter(|t| t.kind.is_test_producing() && t.harness == Some(false))
        .collect()
}

/// Scan every workspace package's manifest for a forged custom harness (item 3).
/// Returns the human-readable identities (`<package> [lib|bin|test] <name?>`) of
/// every test-producing target that set `harness = false`. An empty result means
/// no forge was found; a manifest that cannot be read is skipped (its absence is
/// itself caught by the enumeration checks — cargo cannot build a target whose
/// manifest is gone).
#[must_use]
pub fn forged_harness_targets(meta: &WorkspaceMetadata) -> Vec<String> {
    let mut forged = Vec::new();
    for pkg in &meta.packages {
        let Ok(manifest) = std::fs::read_to_string(&pkg.manifest_path) else {
            continue;
        };
        for t in forged_harness_in_manifest(&manifest) {
            let kind = match t.kind {
                ManifestKind::Lib => "lib",
                ManifestKind::Bin => "bin",
                ManifestKind::Test => "test",
                ManifestKind::Bench => "bench",
                ManifestKind::Example => "example",
            };
            let name = t.name.as_deref().unwrap_or("<default>");
            forged.push(format!("{} [{kind}] {name}", pkg.name));
        }
    }
    forged
}

/// Derive the confident set of expected test-target keys from trusted metadata
/// (item 2). Only the deterministically test-producing kinds `lib` and `test`
/// are included, and a target is **excluded** when its manifest table disables it
/// (`test = false` or `harness = false`) — so the set is a conservative *lower
/// bound* on what `cargo test` must enumerate. Requiring the captured enumeration
/// to be a superset of it therefore never false-blocks a legitimately-disabled
/// target, while still catching a compromised/empty enumeration.
///
/// `read_manifest` is injected so the derivation stays pure and unit-testable;
/// [`expected_test_targets`] wraps it over the filesystem. A package whose
/// manifest cannot be read contributes **no** requirements (fail-open for that
/// package, so a genuinely-unreadable manifest never false-blocks — the global
/// empty-enumeration check still covers a gross compromise).
fn expected_test_targets_with<F>(meta: &WorkspaceMetadata, mut read_manifest: F) -> BTreeSet<String>
where
    F: FnMut(&Path) -> Option<String>,
{
    let mut expected = BTreeSet::new();
    for pkg in &meta.packages {
        let Some(manifest) = read_manifest(&pkg.manifest_path) else {
            continue;
        };
        let mtargets = parse_manifest_targets(&manifest);
        for t in &pkg.targets {
            let kind = t.primary_kind();
            if kind != "lib" && kind != "test" {
                continue;
            }
            // A target gated behind required-features is not built by a
            // default-feature `cargo test`, so requiring it would false-block
            // (review follow-up). Exclude it — the confident set stays a lower
            // bound on what the default build must enumerate.
            if !t.required_features.is_empty() {
                continue;
            }
            let mkind = if kind == "lib" {
                ManifestKind::Lib
            } else {
                ManifestKind::Test
            };
            // A manifest table for THIS target that disables it → exclude.
            let disabled = mtargets.iter().any(|mt| {
                mt.kind == mkind
                    && manifest_matches_target(mt, &t.name)
                    && (mt.test == Some(false) || mt.harness == Some(false))
            });
            if !disabled {
                expected.insert(target_key(&pkg.name, kind, &t.name));
            }
        }
    }
    expected
}

/// Whether a manifest target table refers to the metadata target named
/// `target_name`. A `[lib]` has no name (one per package) and always matches; an
/// array-of-tables entry matches iff its declared `name` equals `target_name`.
fn manifest_matches_target(mt: &ManifestTarget, target_name: &str) -> bool {
    match (mt.kind, &mt.name) {
        (ManifestKind::Lib, _) => true,
        (_, Some(name)) => name == target_name,
        (_, None) => false,
    }
}

/// Filesystem wrapper over [`expected_test_targets_with`] — the confident
/// expected test-target set for a real workspace.
#[must_use]
pub fn expected_test_targets(meta: &WorkspaceMetadata) -> BTreeSet<String> {
    expected_test_targets_with(meta, |p| std::fs::read_to_string(p).ok())
}

/// True when metadata reports at least one test-producing target (`lib` / `test`
/// / `bin`) anywhere in the workspace — used to decide whether an **empty**
/// captured enumeration is anomalous (item 2: fail closed on empty when tests
/// exist).
#[must_use]
pub fn has_test_targets(meta: &WorkspaceMetadata) -> bool {
    meta.packages.iter().any(|p| {
        p.targets
            .iter()
            .any(|t| matches!(t.primary_kind(), "lib" | "test" | "bin"))
    })
}

/// Reject the capture outright if any workspace manifest declares a forged
/// custom harness (item 3 / F5). A test-producing target with `harness = false`
/// can print perfectly balanced forged libtest output, so the floor refuses to
/// trust the whole capture rather than let one target launder arbitrary passes.
///
/// A manifest that reads but **does not parse** as TOML also fails closed (review
/// follow-up): `cargo metadata` proved the package exists, so an unparseable
/// `Cargo.toml` is anomalous — and letting it through would let an adversary hide
/// a `harness = false` behind TOML the floor's parser chokes on but cargo accepts.
/// A manifest that cannot be *read* is skipped (its absence is caught by the
/// enumeration checks — cargo cannot build a target whose manifest is gone).
pub fn reject_forged_harness(meta: &WorkspaceMetadata) -> Result<(), FloorError> {
    let mut forged = Vec::new();
    for pkg in &meta.packages {
        let Ok(manifest) = std::fs::read_to_string(&pkg.manifest_path) else {
            continue;
        };
        if toml::from_str::<RawManifest>(&manifest).is_err() {
            return Err(FloorError::Capture {
                what: "tests",
                message: format!(
                    "package {}'s Cargo.toml at {} is unparseable; refusing to certify a capture \
                     whose custom-harness declarations cannot be verified (failing closed)",
                    pkg.name,
                    pkg.manifest_path.display()
                ),
            });
        }
        for t in forged_harness_in_manifest(&manifest) {
            let kind = match t.kind {
                ManifestKind::Lib => "lib",
                ManifestKind::Bin => "bin",
                ManifestKind::Test => "test",
                ManifestKind::Bench => "bench",
                ManifestKind::Example => "example",
            };
            let name = t.name.as_deref().unwrap_or("<default>");
            forged.push(format!("{} [{kind}] {name}", pkg.name));
        }
    }
    if forged.is_empty() {
        Ok(())
    } else {
        Err(FloorError::Capture {
            what: "tests",
            message: format!(
                "forged custom test harness (`harness = false`) on test-producing target(s): {}; \
                 failing closed (a `[[bench]] harness = false` is allowed, these are not)",
                forged.join(", ")
            ),
        })
    }
}

/// Verify the captured enumeration against the independent metadata manifest
/// (item 2). Fails closed when metadata shows test-producing targets but the
/// captured set is **empty** (a compromised/already-empty enumeration), and when
/// the captured set is missing any target in the confident expected set (a
/// narrowing that predates the fork, which the baseline-relative superset gate
/// cannot see). `captured` is [`super::snapshot::TestSnapshot::targets`].
pub fn verify_enumeration(
    meta: &WorkspaceMetadata,
    captured: &BTreeSet<String>,
) -> Result<(), FloorError> {
    if captured.is_empty() && has_test_targets(meta) {
        return Err(FloorError::Capture {
            what: "tests",
            message: "captured test enumeration is empty but `cargo metadata` reports \
                      test-producing targets; failing closed (compromised/empty enumeration)"
                .into(),
        });
    }
    let expected = expected_test_targets(meta);
    let missing: Vec<&String> = expected.difference(captured).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(FloorError::Capture {
            what: "tests",
            message: format!(
                "captured enumeration is missing metadata-expected test target(s): {}; \
                 failing closed",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
    }
}

/// The lib target name for a package, if it has one — the doctest pass runs only
/// for packages with a library target (rustdoc doctests come from the lib).
#[must_use]
pub fn lib_target_name(pkg: &PackageMeta) -> Option<String> {
    pkg.targets
        .iter()
        .find(|t| t.primary_kind() == "lib")
        .map(|t| t.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_from(json: &str) -> WorkspaceMetadata {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parse_manifest_reads_target_flags() {
        let m = r#"
            [package]
            name = "p"
            [lib]
            test = false
            [[test]]
            name = "e2e"
            [[bench]]
            name = "crit"
            harness = false
            [[bin]]
            name = "cli"
            harness = false
        "#;
        let ts = parse_manifest_targets(m);
        assert!(ts.contains(&ManifestTarget {
            kind: ManifestKind::Lib,
            name: None,
            harness: None,
            test: Some(false),
        }));
        assert!(ts.contains(&ManifestTarget {
            kind: ManifestKind::Bench,
            name: Some("crit".into()),
            harness: Some(false),
            test: None,
        }));
    }

    #[test]
    fn forged_harness_flags_test_producing_but_allows_bench() {
        let m = r#"
            [package]
            name = "p"
            [[bench]]
            name = "crit"
            harness = false
            [[test]]
            name = "e2e"
            harness = false
            [lib]
            harness = false
        "#;
        let forged = forged_harness_in_manifest(m);
        // The bench is allowed; the test and lib are forged.
        assert_eq!(forged.len(), 2);
        assert!(forged.iter().any(|t| t.kind == ManifestKind::Test));
        assert!(forged.iter().any(|t| t.kind == ManifestKind::Lib));
        assert!(!forged.iter().any(|t| t.kind == ManifestKind::Bench));
    }

    #[test]
    fn forged_harness_ignores_declared_true_and_unset() {
        let m = r#"
            [package]
            name = "p"
            [lib]
            harness = true
            [[test]]
            name = "e2e"
        "#;
        assert!(forged_harness_in_manifest(m).is_empty());
    }

    #[test]
    fn expected_targets_includes_lib_and_test_excludes_disabled() {
        let meta = meta_from(
            r#"{"packages":[
              {"name":"pkgA","manifest_path":"/A/Cargo.toml","targets":[
                 {"name":"pkgA","kind":["lib"]},
                 {"name":"e2e","kind":["test"]},
                 {"name":"cli","kind":["bin"]},
                 {"name":"crit","kind":["bench"]}
              ]}
            ]}"#,
        );
        // Manifest disables the e2e integration test via test = false.
        let expected = expected_test_targets_with(&meta, |_| {
            Some(
                r#"[package]
                   name = "pkgA"
                   [[test]]
                   name = "e2e"
                   test = false"#
                    .to_string(),
            )
        });
        assert!(expected.contains("pkgA/lib/pkgA"));
        assert!(!expected.contains("pkgA/test/e2e"), "test=false excluded");
        // bin and bench are never in the confident set.
        assert!(!expected.iter().any(|k| k.contains("/bin/")));
        assert!(!expected.iter().any(|k| k.contains("/bench/")));
    }

    #[test]
    fn expected_targets_skips_unreadable_manifest_without_false_block() {
        let meta = meta_from(
            r#"{"packages":[
              {"name":"pkgA","manifest_path":"/A/Cargo.toml","targets":[
                 {"name":"pkgA","kind":["lib"]}
              ]}
            ]}"#,
        );
        // Manifest unreadable → the package contributes no requirements.
        let expected = expected_test_targets_with(&meta, |_| None);
        assert!(expected.is_empty());
    }

    #[test]
    fn has_test_targets_detects_presence() {
        let with = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/p","targets":[{"name":"p","kind":["lib"]}]}]}"#,
        );
        assert!(has_test_targets(&with));
        let without = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/p","targets":[{"name":"p","kind":["custom-build"]}]}]}"#,
        );
        assert!(!has_test_targets(&without));
    }

    #[test]
    fn verify_enumeration_fails_closed_on_empty_when_tests_exist() {
        let meta = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/p","targets":[{"name":"p","kind":["lib"]}]}]}"#,
        );
        let empty = BTreeSet::new();
        assert!(verify_enumeration(&meta, &empty).is_err());
        // A workspace with no test-producing targets tolerates an empty capture.
        let no_tests = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/p","targets":[{"name":"p","kind":["custom-build"]}]}]}"#,
        );
        assert!(verify_enumeration(&no_tests, &empty).is_ok());
    }

    #[test]
    fn verify_enumeration_requires_expected_superset() {
        let meta = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/nonexistent-Cargo.toml","targets":[
                 {"name":"p","kind":["lib"]}]}]}"#,
        );
        // manifest_path is unreadable, so expected is empty → any non-empty
        // capture passes (fail-open per package, no false block).
        let captured: BTreeSet<String> = ["p/lib/p".to_string()].into_iter().collect();
        assert!(verify_enumeration(&meta, &captured).is_ok());
    }

    #[test]
    fn reject_forged_harness_errs_on_unreadable_ok() {
        // No manifests readable → no forge detected → Ok (the enumeration checks
        // catch a vanished manifest separately).
        let meta = meta_from(
            r#"{"packages":[{"name":"p","manifest_path":"/no/such/Cargo.toml","targets":[]}]}"#,
        );
        assert!(reject_forged_harness(&meta).is_ok());
    }

    #[test]
    fn reject_forged_harness_fails_closed_on_unparseable_manifest() {
        // A readable but unparseable Cargo.toml fails closed (review follow-up):
        // an adversary could otherwise hide `harness = false` behind TOML the
        // floor's parser rejects but cargo accepts.
        let dir = tempfile::TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "this is [not valid toml == ").unwrap();
        let meta = meta_from(&format!(
            r#"{{"packages":[{{"name":"p","manifest_path":"{}","targets":[]}}]}}"#,
            manifest.display()
        ));
        let err = reject_forged_harness(&meta).unwrap_err();
        assert!(format!("{err}").contains("unparseable"), "{err}");
    }

    #[test]
    fn expected_targets_excludes_required_features_targets() {
        // A target gated behind required-features is not built by a default-feature
        // `cargo test`, so it must not be required (else false-block).
        let meta = meta_from(
            r#"{"packages":[
              {"name":"p","manifest_path":"/p/Cargo.toml","targets":[
                 {"name":"p","kind":["lib"]},
                 {"name":"gpu","kind":["test"],"required-features":["gpu"]}
              ]}
            ]}"#,
        );
        let expected =
            expected_test_targets_with(&meta, |_| Some("[package]\nname = \"p\"".to_string()));
        assert!(expected.contains("p/lib/p"));
        assert!(
            !expected.contains("p/test/gpu"),
            "required-features excluded"
        );
    }

    #[test]
    fn package_names_are_sorted_and_unique() {
        let meta = meta_from(
            r#"{"packages":[
              {"name":"b","manifest_path":"/b","targets":[]},
              {"name":"a","manifest_path":"/a","targets":[]}
            ]}"#,
        );
        let names: Vec<String> = meta.package_names().into_iter().collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }
}
