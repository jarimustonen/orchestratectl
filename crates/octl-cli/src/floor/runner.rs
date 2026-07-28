//! The floor's capture layer (design.md §4) — impure functions that run
//! checks/tests/clippy and read files, then hand pure values to the
//! [`super::gates`]. Everything mechanical here shells out; the parsing it
//! delegates to [`super::parse`] is pure and separately tested.
//!
//! # Trust posture (`floor-capture-trust-model`)
//!
//! - **Structured, not text.** Clippy is captured via `--message-format=json`
//!   and keyed by lint code; tests are enumerated as `compiler-artifact`
//!   executables and run one binary at a time, so each observation is bound to
//!   its `(package, target)`.
//! - **Fail-closed.** A capture that cannot prove complete compilation +
//!   execution is a [`FloorError`], never a silently-empty snapshot that passes
//!   gates vacuously: unparseable cargo JSON, a missing `build-finished`, an
//!   `error`-level diagnostic, a non-zero process exit *despite* a
//!   `build-finished` record (JSON-injection-then-kill), a libtest binary whose
//!   parsed counts disagree with its announced summary, a non-zero `filtered
//!   out` count (a leaked filter → subset), or an exit code inconsistent with
//!   that summary all reject.
//! - **Execution isolation.** Capture subprocesses run under
//!   [`isolated_command`] — `env_clear()` + a small allow-list — so an inherited
//!   `RUSTFLAGS`/`RUSTDOCFLAGS`/`RUSTC_WRAPPER` cannot change the observed
//!   warning/test set. (Plan-declared `checks` in [`run_check`] are *not*
//!   isolated — they are the caller's own contract, run verbatim.)
//!
//! In-repo `.cargo/config.toml` / `rust-toolchain.toml` overrides are not fully
//! neutralized here (that needs `--config`/out-of-tree invocation); the
//! toolchain actually used is recorded via [`rustc_version`] as provenance on
//! the baseline, and full neutralization is deferred to the follow-up issue.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use octl_core::plan;

use super::git;
use super::parse::{
    self, count_assert_macros, parse_cargo_stream, parse_libtest_report, reconcile_single_binary,
};
use super::snapshot::{CheckRun, ClippySnapshot, TestSnapshot};
use super::FloorError;

/// Environment variables the capture subprocess is allowed to inherit. Anything
/// not on this list — notably `RUSTFLAGS`, `RUSTDOCFLAGS`,
/// `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `RUSTC_WRAPPER` — is
/// dropped by `env_clear()` and never reaches cargo/rustc.
const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "TERM",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "XDG_CACHE_HOME",
    "GIT_BIN",
    // Dynamic-linker search paths, copied from the (trusted) parent env so a
    // dynamically-linked test binary can still find its own dylibs; dropping
    // them would fail-closed on a valid suite (a DoS), and they come from the
    // orchestrator's env, not the repo under review.
    "LD_LIBRARY_PATH",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
];

/// A [`Command`] for a capture subprocess with a cleared, allow-listed
/// environment (see [`ENV_ALLOWLIST`]). `LC_ALL=C` stabilizes any message we
/// must read; `CARGO_TERM_COLOR=never` keeps JSON clean of ANSI escapes;
/// `CARGO_INCREMENTAL=0` stops incremental compilation from skipping a fresh
/// lint pass (a cached incremental unit can re-use a prior compile and NOT
/// re-emit its warnings — see the shared-`target/` caching caveat in the module
/// docs).
fn isolated_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    for key in ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.env("LC_ALL", "C");
    cmd.env("CARGO_TERM_COLOR", "never");
    cmd.env("CARGO_INCREMENTAL", "0");
    cmd
}

/// The active `rustc -V` string (isolated), or `"unknown"` if it cannot be
/// determined — recorded as baseline provenance so a snapshot captured under a
/// different toolchain than the tip is detectable.
#[must_use]
pub fn rustc_version(cwd: &Path) -> String {
    isolated_command("rustc")
        .arg("-V")
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

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
///
/// Checks are **not** run under [`isolated_command`]: they are the plan's own
/// declared commands and may legitimately depend on ambient env; isolation is
/// applied to the floor's *own* clippy/test capture, not the caller's checks.
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

/// Capture a target-qualified, fail-closed [`TestSnapshot`] by running the test
/// suite as structured, per-binary libtest.
///
/// `test_cmd` is the base cargo test invocation (e.g. `cargo test`,
/// `cargo test --workspace`). Two isolated phases:
///
/// 1. **Enumerate** — `<test_cmd> --no-run --message-format=json` builds every
///    test harness and reports its executable + `(package, target)`. The stream
///    must parse, carry a successful `build-finished`, and be free of
///    `error`-level diagnostics; otherwise this fails closed (a compile failure
///    or bad flag must NOT yield an empty snapshot that passes gates).
/// 2. **Run** — each enumerated binary is executed directly (isolated) and its
///    libtest text is reconciled against its own announced summary
///    ([`reconcile_single_binary`]). A forged `test x ... ok` line, a truncated
///    run, or an exit code inconsistent with the summary rejects the whole
///    capture. Surviving names become target-qualified [`TestId`]s.
///
/// Doctests (run by rustdoc, not a `compiler-artifact` binary) are **not**
/// captured here; qualifying them needs the nightly libtest JSON format and is
/// deferred to the follow-up issue.
pub fn capture_test_snapshot(test_cmd: &str, cwd: &Path) -> Result<TestSnapshot, FloorError> {
    let enumerate_cmd = inject_cargo_flags(test_cmd, &["--no-run", "--message-format=json"]);
    let out = isolated_command("sh")
        .arg("-c")
        .arg(&enumerate_cmd)
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "tests",
            message: format!("could not run `{enumerate_cmd}`: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let messages = parse_cargo_stream(&stdout).map_err(|e| FloorError::Capture {
        what: "tests",
        message: format!("test enumeration produced unparseable cargo output: {e}"),
    })?;

    // Fail closed: the build must have compiled cleanly and finished.
    if parse::has_compile_error(&messages) {
        return Err(FloorError::Capture {
            what: "tests",
            message: "test build reported a compile error; refusing an empty/partial snapshot"
                .into(),
        });
    }
    match parse::build_finished(&messages) {
        Some(true) => {}
        Some(false) => {
            return Err(FloorError::Capture {
                what: "tests",
                message: format!(
                    "`{enumerate_cmd}` reported build failure (exit {:?}); failing closed",
                    out.status.code()
                ),
            });
        }
        None => {
            return Err(FloorError::Capture {
                what: "tests",
                message: format!(
                    "`{enumerate_cmd}` produced no build-finished record (truncated?); failing closed. stderr: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            });
        }
    }

    // Even with a `build-finished:true` record, the process itself must have
    // exited 0. A proc-macro/build script can inject `{"reason":"build-finished",
    // "success":true}` on stdout and then SIGKILL cargo — the JSON would look
    // complete while the build was actually truncated. A real `cargo test
    // --no-run` that compiles cleanly exits 0, so this closes that hole.
    if !out.status.success() {
        return Err(FloorError::Capture {
            what: "tests",
            message: format!(
                "`{enumerate_cmd}` exited {:?} despite a build-finished record (killed/truncated?); failing closed",
                out.status.code()
            ),
        });
    }

    let binaries = parse::test_binaries(&messages);
    let mut snap = TestSnapshot::default();
    for bin in &binaries {
        run_one_test_binary(bin, cwd, &mut snap)?;
    }
    Ok(snap)
}

/// Run one enumerated test binary, reconcile it, and fold its target-qualified
/// ids into `snap`. Any inconsistency is a fail-closed [`FloorError`].
fn run_one_test_binary(
    bin: &parse::TestBinary,
    cwd: &Path,
    snap: &mut TestSnapshot,
) -> Result<(), FloorError> {
    let out = isolated_command(&bin.executable)
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "tests",
            message: format!("could not run test binary {}: {e}", bin.executable),
        })?;

    let combined = join_streams(&out.stdout, &out.stderr);
    let report = parse_libtest_report(&combined);
    let summary = reconcile_single_binary(&report).map_err(|d| FloorError::Capture {
        what: "tests",
        message: format!(
            "test binary {} ({}/{}): untrustworthy output: {d}",
            bin.executable, bin.target_kind, bin.target
        ),
    })?;

    // libtest exits 0 iff nothing failed. An exit code that disagrees with the
    // announced summary is an anomaly (a panic outside a test, a tampered exit)
    // → fail closed.
    let exit_ok = out.status.code() == Some(0);
    if exit_ok != (summary.failed == 0) {
        return Err(FloorError::Capture {
            what: "tests",
            message: format!(
                "test binary {} exit code {:?} inconsistent with summary ({} failed); failing closed",
                bin.executable,
                out.status.code(),
                summary.failed
            ),
        });
    }

    snap.passed.extend(parse::qualify(
        &bin.package,
        &bin.target_kind,
        &bin.target,
        &report.passed,
    ));
    snap.failed.extend(parse::qualify(
        &bin.package,
        &bin.target_kind,
        &bin.target,
        &report.failed,
    ));
    snap.ignored.extend(parse::qualify(
        &bin.package,
        &bin.target_kind,
        &bin.target,
        &report.ignored,
    ));
    Ok(())
}

/// Capture a structured [`ClippySnapshot`] from `cargo clippy`
/// `--message-format=json`. Warnings are read from the JSON records keyed by
/// lint code — a `println!`/`build.rs` cannot fabricate one.
///
/// Fail-closed: the stream must parse, carry a terminal `build-finished`, be
/// free of `error`-level diagnostics, and the process must have exited 0. A
/// clippy run over compilable code exits 0 and emits its warnings at
/// `level: "warning"`; a diagnostic promoted to `error` (a real compile error,
/// or a `deny`-level lint from `[lints]`/`#![deny]`/`-D`) means the code is not
/// in a clean, gateable state, so the capture is rejected rather than trusting a
/// partial warning set.
pub fn capture_clippy_snapshot(clippy_cmd: &str, cwd: &Path) -> Result<ClippySnapshot, FloorError> {
    let cmd = inject_cargo_flags(clippy_cmd, &["--message-format=json"]);
    let out = isolated_command("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(cwd)
        .output()
        .map_err(|e| FloorError::Capture {
            what: "clippy",
            message: format!("could not run `{cmd}`: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let messages = parse_cargo_stream(&stdout).map_err(|e| FloorError::Capture {
        what: "clippy",
        message: format!("clippy produced unparseable cargo output: {e}"),
    })?;

    if parse::has_compile_error(&messages) {
        return Err(FloorError::Capture {
            what: "clippy",
            message: "clippy reported an error-level diagnostic; refusing a partial warning set"
                .into(),
        });
    }
    if parse::build_finished(&messages).is_none() {
        return Err(FloorError::Capture {
            what: "clippy",
            message: format!(
                "`{cmd}` produced no build-finished record (truncated?); failing closed. stderr: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    // A clean clippy run over compilable code exits 0; a non-zero exit alongside
    // a `build-finished` record means the process was killed after injecting the
    // record, or the build genuinely failed — either way, fail closed.
    if !out.status.success() {
        return Err(FloorError::Capture {
            what: "clippy",
            message: format!(
                "`{cmd}` exited {:?} despite a build-finished record (killed/failed?); failing closed",
                out.status.code()
            ),
        });
    }

    Ok(ClippySnapshot {
        warnings: parse::clippy_warnings(&messages),
    })
}

/// Rewrite a cargo command to carry the floor's required `flags`, robust to a
/// `--` argument separator: any existing `--message-format[=x]` token is dropped
/// (the floor always forces its own), and the injected flags are inserted
/// **before** the first `--` (or appended if none). Blindly appending would put
/// the flags after `--`, where cargo passes them to the *test binary* instead of
/// to cargo — breaking `--no-run` and crashing the harness on an unknown flag.
/// Token-based on whitespace: a base command with quoted, space-containing args
/// is out of contract (the floor's own commands never have them).
fn inject_cargo_flags(cmd: &str, flags: &[&str]) -> String {
    let mut before: Vec<&str> = Vec::new();
    let mut after: Vec<&str> = Vec::new();
    let mut past_sep = false;
    let mut skip_next = false;
    for tok in cmd.split_whitespace() {
        if past_sep {
            after.push(tok);
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        if tok == "--" {
            past_sep = true;
            after.push(tok);
            continue;
        }
        if tok == "--message-format" {
            skip_next = true; // drop the following value token too
            continue;
        }
        if tok.starts_with("--message-format=") {
            continue;
        }
        before.push(tok);
    }
    before.extend_from_slice(flags);
    before.extend(after);
    before.join(" ")
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

/// Count `assert*!` macros in each of `files` as of git `r#ref`.
///
/// The ref is first pinned to an immutable commit OID
/// ([`git::resolve_commit`]) and every file is read at that OID, so the whole
/// map is provably captured at one commit (not re-resolving a mutable ref
/// per-file) — the assertion-count provenance binding of
/// `floor-capture-trust-model` item 5. A file absent at that commit is simply
/// omitted (no baseline to regress from); a bad ref is a hard [`FloorError`].
pub fn assertion_counts_at_ref(
    repo: &Path,
    r#ref: &str,
    files: &[PathBuf],
) -> Result<BTreeMap<PathBuf, usize>, FloorError> {
    let commit = git::resolve_commit(repo, r#ref)?;
    let mut counts = BTreeMap::new();
    for f in files {
        if let Some(content) = git::file_at_ref(repo, &commit, f)? {
            counts.insert(f.clone(), count_assert_macros(&content));
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
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

    /// Write an executable shell script and return its path.
    fn write_script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path.to_string_lossy().into_owned()
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
    fn run_check_honors_expect_exit_and_cwd() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut c = check("runs in sub, exits 2", "exit 2");
        c.cwd = Some("sub".to_string());
        c.expect_exit = Some(2);
        let r = run_check(&c, dir.path());
        assert!(r.passed);
        assert_eq!(r.cwd.as_deref(), Some("sub"));
    }

    #[test]
    fn run_checks_preserves_order() {
        let dir = TempDir::new().unwrap();
        let rs = run_checks(&[check("a", "exit 0"), check("b", "exit 1")], dir.path());
        assert_eq!(rs.len(), 2);
        assert!(rs[0].passed);
        assert!(!rs[1].passed);
    }

    #[test]
    fn isolated_command_clears_rustflags_but_keeps_path() {
        // The isolation guarantee (item 4): a poisoned RUSTFLAGS in the parent
        // env does not reach the capture subprocess, but PATH survives so cargo
        // is still findable.
        std::env::set_var("RUSTFLAGS", "--cfg poisoned");
        let dir = TempDir::new().unwrap();
        let out = isolated_command("sh")
            .arg("-c")
            .arg("echo \"RUSTFLAGS=[${RUSTFLAGS:-}] PATH_SET=${PATH:+yes}\"")
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::env::remove_var("RUSTFLAGS");
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("RUSTFLAGS=[]"), "RUSTFLAGS leaked: {s}");
        assert!(s.contains("PATH_SET=yes"), "PATH missing: {s}");
    }

    #[test]
    fn inject_cargo_flags_replaces_message_format() {
        assert_eq!(
            inject_cargo_flags(
                "cargo clippy --message-format=short",
                &["--message-format=json"]
            ),
            "cargo clippy --message-format=json"
        );
        assert_eq!(
            inject_cargo_flags(
                "cargo clippy --message-format short --workspace",
                &["--message-format=json"]
            ),
            "cargo clippy --workspace --message-format=json"
        );
        assert_eq!(
            inject_cargo_flags("cargo clippy", &["--message-format=json"]),
            "cargo clippy --message-format=json"
        );
    }

    #[test]
    fn inject_cargo_flags_inserts_before_the_arg_separator() {
        // Flags meant for cargo must land BEFORE `--`; everything after `--`
        // belongs to the test binary. A naive append would break `--no-run`.
        assert_eq!(
            inject_cargo_flags(
                "cargo test --workspace -- --ignored",
                &["--no-run", "--message-format=json"]
            ),
            "cargo test --workspace --no-run --message-format=json -- --ignored"
        );
        // An existing --message-format after `--` (a driver flag) is left alone;
        // only cargo-side ones are stripped.
        assert_eq!(
            inject_cargo_flags("cargo test -- --nocapture", &["--message-format=json"]),
            "cargo test --message-format=json -- --nocapture"
        );
    }

    #[test]
    fn clippy_capture_parses_json_and_strips_span() {
        // Drive capture through a fake `cargo` on stdout — no toolchain needed.
        // The script ignores the appended flags and prints a fixed JSON stream.
        let dir = TempDir::new().unwrap();
        let json = concat!(
            r#"{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"warning","message":"unused variable: `x`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/a.rs","line_start":3,"is_primary":true}]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\n"
        );
        let script = write_script(
            dir.path(),
            "fakeclippy",
            &format!("#!/bin/sh\nprintf '%s' '{json}'\n"),
        );
        let snap = capture_clippy_snapshot(&script, dir.path()).unwrap();
        assert_eq!(snap.warnings.len(), 1);
        let w = snap.warnings.iter().next().unwrap();
        assert_eq!(w.lint, "unused_variables");
        assert_eq!(w.file, "src/a.rs");
    }

    #[test]
    fn clippy_capture_fails_closed_on_compile_error() {
        // done-criteria (b): a compile error must fail closed, not yield an
        // empty (vacuously-passing) warning set.
        let dir = TempDir::new().unwrap();
        let json = concat!(
            r#"{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":false}"#,
            "\n"
        );
        let script = write_script(
            dir.path(),
            "fakeclippy",
            &format!("#!/bin/sh\nprintf '%s' '{json}'\nexit 101\n"),
        );
        let err = capture_clippy_snapshot(&script, dir.path()).unwrap_err();
        assert!(format!("{err}").contains("error-level diagnostic"), "{err}");
    }

    #[test]
    fn clippy_capture_fails_closed_on_unparseable_output() {
        let dir = TempDir::new().unwrap();
        // A bad flag / non-JSON babble on stdout ⇒ reject.
        let script = write_script(
            dir.path(),
            "fakeclippy",
            "#!/bin/sh\necho 'error: unknown flag'\n",
        );
        assert!(capture_clippy_snapshot(&script, dir.path()).is_err());
    }

    #[test]
    fn test_capture_enumerates_runs_and_qualifies() {
        // A fake `cargo` emits a compiler-artifact pointing at a second fake
        // binary that emits libtest text — the whole capture path with no real
        // toolchain. The resulting id is target-qualified.
        let dir = TempDir::new().unwrap();
        let libtest = write_script(
            dir.path(),
            "faketest",
            "#!/bin/sh\nprintf 'test mymod::works ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\\n'\n",
        );
        let artifact = format!(
            r#"{{"reason":"compiler-artifact","package_id":"p#octl-cli@0.1.0","target":{{"name":"octl-cli","kind":["lib"]}},"profile":{{"test":true}},"executable":"{libtest}"}}"#
        );
        let stream = format!("{artifact}\n{{\"reason\":\"build-finished\",\"success\":true}}\n");
        let cargo = write_script(
            dir.path(),
            "fakecargo",
            &format!("#!/bin/sh\nprintf '%s' '{stream}'\n"),
        );
        let snap = capture_test_snapshot(&cargo, dir.path()).unwrap();
        assert_eq!(snap.passed.len(), 1);
        let id = snap.passed.iter().next().unwrap();
        assert_eq!(id.package, "octl-cli");
        assert_eq!(id.target_kind, "lib");
        assert_eq!(id.name, "mymod::works");
    }

    #[test]
    fn test_capture_rejects_forged_ok_line() {
        // done-criteria (a): a test binary that prints an extra `test x ... ok`
        // beyond its announced summary is rejected — the forged pass never
        // becomes a TestId.
        let dir = TempDir::new().unwrap();
        let libtest = write_script(
            dir.path(),
            "faketest",
            "#!/bin/sh\nprintf 'test real::actual ... ok\\ntest forged::injected ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\\n'\n",
        );
        let artifact = format!(
            r#"{{"reason":"compiler-artifact","package_id":"p#pkg@0.1.0","target":{{"name":"pkg","kind":["lib"]}},"profile":{{"test":true}},"executable":"{libtest}"}}"#
        );
        let stream = format!("{artifact}\n{{\"reason\":\"build-finished\",\"success\":true}}\n");
        let cargo = write_script(
            dir.path(),
            "fakecargo",
            &format!("#!/bin/sh\nprintf '%s' '{stream}'\n"),
        );
        let err = capture_test_snapshot(&cargo, dir.path()).unwrap_err();
        assert!(format!("{err}").contains("untrustworthy"), "{err}");
    }

    #[test]
    fn test_capture_fails_closed_on_injected_build_finished_then_nonzero_exit() {
        // Adversary injects a valid `build-finished:true` on stdout but the
        // process is killed / exits non-zero (a truncated build masquerading as
        // complete). The JSON looks fine, but the exit-status guard rejects it.
        let dir = TempDir::new().unwrap();
        let cargo = write_script(
            dir.path(),
            "fakecargo",
            "#!/bin/sh\nprintf '%s\\n' '{\"reason\":\"build-finished\",\"success\":true}'\nexit 137\n",
        );
        let err = capture_test_snapshot(&cargo, dir.path()).unwrap_err();
        assert!(format!("{err}").contains("killed/truncated"), "{err}");
    }

    #[test]
    fn test_capture_fails_closed_on_build_failure() {
        // done-criteria (b): a build failure during enumeration fails closed.
        let dir = TempDir::new().unwrap();
        let stream = concat!(
            r#"{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"error","message":"boom","code":{"code":"E0308"},"spans":[]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":false}"#,
            "\n"
        );
        let cargo = write_script(
            dir.path(),
            "fakecargo",
            &format!("#!/bin/sh\nprintf '%s' '{stream}'\nexit 101\n"),
        );
        assert!(capture_test_snapshot(&cargo, dir.path()).is_err());
    }

    #[test]
    fn test_capture_fails_closed_on_no_build_finished() {
        let dir = TempDir::new().unwrap();
        // Valid JSON but truncated (no build-finished) ⇒ reject.
        let cargo = write_script(
            dir.path(),
            "fakecargo",
            r#"#!/bin/sh
printf '%s\n' '{"reason":"compiler-artifact","package_id":"p#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"profile":{"test":false},"executable":null}'
"#,
        );
        assert!(capture_test_snapshot(&cargo, dir.path()).is_err());
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
        assert_eq!(counts[&PathBuf::from("missing.rs")], 0);
    }
}
