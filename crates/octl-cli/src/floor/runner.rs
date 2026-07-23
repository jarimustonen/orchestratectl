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

/// Run one [`plan::Check`] rooted at `cwd` and capture its result.
///
/// **Check-run contract (`plan-check-run-contract`, owner-locked 2026-07-23).**
/// A [`plan::Check`] carries the general goal ([`plan::Check::desc`]) plus a
/// flexible shell command ([`plan::Check::run`]) executed via `sh -c`, with
/// optional precision: [`plan::Check::cwd`] (a repo-relative working directory,
/// joined onto `cwd`; absent = the worktree root) and
/// [`plan::Check::expect_exit`] (the exit code that counts as a pass; absent =
/// 0). The check passes iff the command runs to completion with that exit code.
/// The gates consume [`CheckRun`], not the raw command — that is the seam.
#[must_use]
pub fn run_check(check: &plan::Check, cwd: &Path) -> CheckRun {
    // `check.cwd` is a validator-vetted safe repo-relative path (or absent), so
    // the join stays inside `cwd`; the plan validator ([`plan::validate_plan`])
    // rejects absolute / `..` / `~` cwds before a plan reaches the floor. This
    // is the same lexical-trust stance the floor takes for `files_touched`.
    let run_dir = match &check.cwd {
        Some(rel) => cwd.join(rel),
        None => cwd.to_path_buf(),
    };
    let expected = check.expect_exit.unwrap_or(0);
    let output = Command::new("sh")
        .arg("-c")
        .arg(&check.run)
        .current_dir(&run_dir)
        .output();
    match output {
        Ok(out) => CheckRun {
            desc: check.desc.clone(),
            run: check.run.clone(),
            cwd: check.cwd.clone(),
            // Pass iff the process exited (not signal-killed) with the expected
            // code. A signalled process has `code() == None`, which never equals
            // `Some(expected)` — so it is a fail, never a silent pass.
            passed: out.status.code() == Some(expected),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        // A command that could not even be spawned is a failed check with no
        // exit code — never a silent pass. Name the run dir so a bad `cwd` is
        // diagnosable.
        Err(e) => CheckRun {
            desc: check.desc.clone(),
            run: check.run.clone(),
            cwd: check.cwd.clone(),
            passed: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to spawn check in {}: {e}", run_dir.display()),
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
            cwd: None,
            expect_exit: None,
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
    fn run_check_honors_expect_exit() {
        let dir = TempDir::new().unwrap();
        // A non-zero expected code: the check passes when the command exits with
        // exactly that code, and fails on any other (including the usual 0).
        let mut c = check("expects 3", "exit 3");
        c.expect_exit = Some(3);
        let r = run_check(&c, dir.path());
        assert!(r.passed, "exit 3 with expect_exit=3 should pass");
        assert_eq!(r.exit_code, Some(3));

        let mut c0 = check("expects 3 but exits 0", "exit 0");
        c0.expect_exit = Some(3);
        let r0 = run_check(&c0, dir.path());
        assert!(!r0.passed, "exit 0 with expect_exit=3 should fail");
        assert_eq!(r0.exit_code, Some(0));
    }

    #[test]
    fn run_check_honors_cwd() {
        let dir = TempDir::new().unwrap();
        // A subdirectory holding a marker file; the check's cwd is joined onto
        // the runner's root, so the command sees the marker only when cwd points
        // at the subdir.
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/marker"), "hi").unwrap();

        let mut c = check("marker present in sub", "test -f marker");
        c.cwd = Some("sub".to_string());
        assert!(run_check(&c, dir.path()).passed);

        // Without cwd the command runs at the root, where marker is absent.
        assert!(!run_check(&check("marker at root", "test -f marker"), dir.path()).passed);
    }

    #[test]
    fn run_check_cwd_and_expect_exit_together() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut c = check("runs in sub, exits 2", "exit 2");
        c.cwd = Some("sub".to_string());
        c.expect_exit = Some(2);
        let r = run_check(&c, dir.path());
        assert!(r.passed);
        assert_eq!(r.exit_code, Some(2));
        // The requested cwd is preserved on the audit record.
        assert_eq!(r.cwd.as_deref(), Some("sub"));
    }

    #[test]
    fn run_check_records_cwd_provenance() {
        let dir = TempDir::new().unwrap();
        // Absent cwd → None on the record; present cwd → echoed. So two checks
        // sharing a `run` but differing in cwd stay distinguishable.
        assert_eq!(run_check(&check("root", "true"), dir.path()).cwd, None);
    }

    #[test]
    fn run_check_bad_cwd_names_run_dir() {
        let dir = TempDir::new().unwrap();
        // A cwd that does not exist cannot be entered: the check fails (never a
        // silent pass) and the resolved run dir is named for diagnosability.
        let mut c = check("missing dir", "true");
        c.cwd = Some("does-not-exist".to_string());
        let r = run_check(&c, dir.path());
        assert!(!r.passed);
        assert_eq!(r.exit_code, None);
        assert!(r.stderr.contains("does-not-exist"), "stderr: {}", r.stderr);
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
