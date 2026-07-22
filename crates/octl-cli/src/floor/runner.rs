//! The floor's capture layer (design.md §4) — impure functions that run
//! checks/tests/clippy and read files, then hand pure values to the
//! [`super::gates`]. Everything mechanical here shells out; the parsing it
//! delegates to [`super::parse`] is pure and separately tested, so this module
//! only needs to be exercised for its process/IO plumbing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use octl_core::plan;

use super::git;
use super::parse::{count_assert_macros, parse_clippy_short, parse_libtest_output};
use super::snapshot::{CheckRun, ClippySnapshot, TestSnapshot};
use super::FloorError;

/// Run one [`plan::Check`] in `cwd` and capture its result.
///
/// **Check-run contract (open decision `plan-check-run-contract`).**
/// [`plan::Check::run`] is today a single shell string; this runner executes it
/// via `sh -c` in `cwd`, exit 0 == pass — the same convention the harness
/// adapter uses for self-checks. When the richer `{cmd, cwd, expect_exit}`
/// contract is settled, only this function changes: the gates consume
/// [`CheckRun`], not the raw command. That is the seam.
#[must_use]
pub fn run_check(check: &plan::Check, cwd: &Path) -> CheckRun {
    let output = Command::new("sh")
        .arg("-c")
        .arg(&check.run)
        .current_dir(cwd)
        .output();
    match output {
        Ok(out) => CheckRun {
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: out.status.success(),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        // A command that could not even be spawned is a failed check with no
        // exit code — never a silent pass.
        Err(e) => CheckRun {
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to spawn check: {e}"),
        },
    }
}

/// Run every check in `cwd`, preserving order.
#[must_use]
pub fn run_checks(checks: &[plan::Check], cwd: &Path) -> Vec<CheckRun> {
    checks.iter().map(|c| run_check(c, cwd)).collect()
}

/// Run `test_cmd` (a shell string, e.g. `cargo test`) in `cwd` and parse its
/// libtest output into a [`TestSnapshot`]. libtest prints outcomes to stdout;
/// stderr is appended too so nothing is missed if a wrapper redirects. A
/// spawn failure is a [`FloorError`] — the caller must not treat "no tests
/// found" as a passing baseline.
pub fn capture_test_snapshot(test_cmd: &str, cwd: &Path) -> Result<TestSnapshot, FloorError> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(test_cmd)
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "tests",
            message: format!("could not run `{test_cmd}`: {e}"),
        })?;
    let combined = join_streams(&out.stdout, &out.stderr);
    Ok(parse_libtest_output(&combined))
}

/// Run `clippy_cmd` (a shell string, e.g.
/// `cargo clippy --message-format=short`) in `cwd` and parse its output into a
/// [`ClippySnapshot`]. clippy writes diagnostics to stderr; stdout is included
/// too for robustness. The command's *exit status is ignored* — clippy exits
/// non-zero under `-D warnings`, but the floor wants the warning *set*, not the
/// pass/fail; a spawn failure is still a [`FloorError`].
pub fn capture_clippy_snapshot(clippy_cmd: &str, cwd: &Path) -> Result<ClippySnapshot, FloorError> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(clippy_cmd)
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "clippy",
            message: format!("could not run `{clippy_cmd}`: {e}"),
        })?;
    let combined = join_streams(&out.stderr, &out.stdout);
    Ok(parse_clippy_short(&combined))
}

/// Concatenate two captured byte streams for line parsing, guaranteeing a
/// newline between them so the last line of `first` and the first line of
/// `second` are never fused into one line (which would corrupt both).
fn join_streams(first: &[u8], second: &[u8]) -> String {
    let mut combined = String::from_utf8_lossy(first).into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&String::from_utf8_lossy(second));
    combined
}

/// Count `assert*!` macros in each of `files` as they exist **on disk** under
/// `cwd` (the current tip). A file that cannot be read (e.g. deleted) counts as
/// zero — a removed test file is itself a density regression, which the gate
/// reports against its non-zero baseline count.
#[must_use]
pub fn assertion_counts_on_disk(cwd: &Path, files: &[PathBuf]) -> BTreeMap<PathBuf, usize> {
    files
        .iter()
        .map(|f| {
            let count = std::fs::read_to_string(cwd.join(f)).map_or(0, |s| count_assert_macros(&s));
            (f.clone(), count)
        })
        .collect()
}

/// Count `assert*!` macros in each of `files` as of git `r#ref` (the baseline
/// fork). A file absent at that ref is simply omitted from the map — it has no
/// baseline to regress from (see [`super::gates::gate_no_test_gaming`]). A git
/// failure other than "path absent" is surfaced.
pub fn assertion_counts_at_ref(
    repo: &Path,
    r#ref: &str,
    files: &[PathBuf],
) -> Result<BTreeMap<PathBuf, usize>, FloorError> {
    let mut counts = BTreeMap::new();
    for f in files {
        if let Some(content) = git::file_at_ref(repo, r#ref, f)? {
            counts.insert(f.clone(), count_assert_macros(&content));
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn check(desc: &str, run: &str) -> plan::Check {
        plan::Check {
            desc: desc.to_string(),
            run: run.to_string(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn run_check_captures_pass_and_fail() {
        let dir = TempDir::new().unwrap();
        let pass = run_check(&check("ok", "exit 0"), dir.path());
        assert!(pass.passed);
        assert_eq!(pass.exit_code, Some(0));

        let fail = run_check(&check("bad", "exit 3"), dir.path());
        assert!(!fail.passed);
        assert_eq!(fail.exit_code, Some(3));
    }

    #[test]
    fn run_check_captures_output_and_cwd() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("marker"), "hi").unwrap();
        // `ls` in cwd sees the marker file ⇒ confirms current_dir plumbing.
        let r = run_check(&check("ls", "ls"), dir.path());
        assert!(r.passed);
        assert!(r.stdout.contains("marker"));
    }

    #[test]
    fn run_checks_preserves_order() {
        let dir = TempDir::new().unwrap();
        let rs = run_checks(
            &[
                check("a", "exit 0"),
                check("b", "exit 1"),
                check("c", "exit 0"),
            ],
            dir.path(),
        );
        assert_eq!(rs.len(), 3);
        assert_eq!(rs[0].desc, "a");
        assert!(!rs[1].passed);
        assert!(rs[2].passed);
    }

    #[test]
    fn capture_test_snapshot_parses_emitted_libtest_lines() {
        let dir = TempDir::new().unwrap();
        // A shell script that emits libtest-shaped lines deterministically —
        // no toolchain needed.
        let cmd = "printf 'test a::x ... ok\\ntest a::y ... FAILED\\n'";
        let snap = capture_test_snapshot(cmd, dir.path()).unwrap();
        assert!(snap.passed.contains("a::x"));
        assert!(snap.failed.contains("a::y"));
    }

    #[test]
    fn capture_clippy_snapshot_parses_stderr_warnings() {
        let dir = TempDir::new().unwrap();
        // clippy writes to stderr; emit there.
        let cmd = "printf 'src/a.rs:1:1: warning: w\\n' 1>&2";
        let snap = capture_clippy_snapshot(cmd, dir.path()).unwrap();
        // Identity has the `:line:col` span normalized away.
        assert!(snap.warnings.contains("src/a.rs: warning: w"));
    }

    #[test]
    fn assertion_counts_on_disk_reads_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "assert!(x); assert_eq!(a,b);").unwrap();
        let counts = assertion_counts_on_disk(
            dir.path(),
            &[PathBuf::from("a.rs"), PathBuf::from("missing.rs")],
        );
        assert_eq!(counts[&PathBuf::from("a.rs")], 2);
        // A missing file counts as zero, still present in the map.
        assert_eq!(counts[&PathBuf::from("missing.rs")], 0);
    }
}
