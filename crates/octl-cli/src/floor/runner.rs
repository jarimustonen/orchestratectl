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
//! - **Floor-pinned target dir (`floor-capture-hardening-round-2` F4).** Every
//!   cargo capture runs with `CARGO_TARGET_DIR` set to a fresh, per-snapshot dir
//!   (env, which beats an in-repo `build.target-dir`; plus a `--target-dir` flag
//!   for an explicit, highest-precedence override — see [`build_cargo_invocation`]).
//!   Baseline and tip therefore never share a warm cache, so `cargo clippy`
//!   cannot re-emit **zero** warnings off a cache the baseline warmed and pass
//!   `gate_no_new_clippy` vacuously.
//!
//! In-repo `.cargo/config.toml` `build.target-dir` is now overridden (above), and
//! a *narrowed* test enumeration (a `test`/`clippy` alias, `--exclude`,
//! `harness = false`, or workspace-narrowing) is caught fail-closed by the F7
//! enumeration-superset gate rather than passing vacuously. The toolchain used is
//! recorded via [`rustc_version`] as baseline provenance.
//!
//! - **Structured argv, sanitized config (`floor-capture-hardening-round-3`
//!   item 1).** The floor no longer composes a repo-influenced `sh -c` string. It
//!   invokes a supervisor-resolved cargo ([`cargo_bin`]) via **argv**
//!   ([`build_cargo_invocation`]) with the sanitizing `--config` overrides of
//!   [`SANITIZING_CONFIG`] (cargo's highest-precedence config layer): repo
//!   `build.rustflags` / `build.rustdocflags` lint-level flips and a
//!   `build.rustc-wrapper` diagnostic-suppressing wrapper are neutralized, and a
//!   repo `[alias] clippy = …` redirect is bypassed by invoking the external
//!   `cargo-clippy` binary directly (`test` is built-in and cannot be aliased).
//!   The old `inject_cargo_flags` whitelist bit-rot and the whitespace/quoting
//!   fragility are gone. A consistently-weakening `rust-toolchain.toml` (an
//!   evil-but-consistent pin) is still only caught as baseline-vs-tip *drift* by
//!   the recorded toolchain, and a repo `[env]`-table `force = true` override of a
//!   compiler env var is not individually rewritten — a fully repo-config-proof
//!   invocation (copy sources into a supervisor-owned tree with a sanitized
//!   `.cargo/config.toml`) is the remaining belt-and-suspenders extreme, tracked
//!   as future work.

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
    "CARGO",
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
/// re-emit its warnings).
///
/// When `target_dir` is `Some`, `CARGO_TARGET_DIR` is pinned to it
/// (`floor-capture-hardening-round-2` item 1 / F4). The env var takes precedence
/// over an in-repo `build.target-dir` in `.cargo/config.toml` (cargo precedence:
/// CLI flag > env > config), so a committed config cannot re-point baseline and
/// tip at one shared warm cache — the bypass where `cargo clippy` on a warm cache
/// re-emits **zero** warnings and `gate_no_new_clippy` passes vacuously. The
/// capture layer passes a *fresh* dir per snapshot (see `capture_snapshot` in
/// the pipeline), so baseline and tip never share a target dir. `env_clear()`
/// already dropped any inherited `CARGO_TARGET_DIR`, so `None` means "cargo's
/// default (`<cwd>/target`, or the repo's `build.target-dir`)" — used only for
/// non-build probes like [`rustc_version`].
pub(crate) fn isolated_command(program: &str, target_dir: Option<&Path>) -> Command {
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
    if let Some(dir) = target_dir {
        cmd.env("CARGO_TARGET_DIR", dir);
    }
    cmd
}

/// The supervisor-resolved cargo binary the floor's own captures invoke
/// (`floor-capture-hardening-round-3` item 1). Prefers the `CARGO` env — set by
/// the toolchain/rustup that launched the supervisor, so it points at the same
/// cargo the orchestrator trusts — and falls back to `cargo` on `PATH`. This is
/// resolved in the *floor* process (trusted parent env), never from the repo
/// under review, so a repo cannot substitute the cargo binary itself. A base
/// command whose first token is literally `cargo` is rewritten to this path; any
/// other first token (a test fixture's fake-cargo script, or an explicit cargo
/// path) is honoured verbatim.
pub(crate) fn cargo_bin() -> String {
    std::env::var("CARGO")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "cargo".to_string())
}

/// cargo's highest-precedence config overrides (`--config KEY=VALUE`, above env
/// and above an in-tree `.cargo/config.toml`) that neutralize the repo-controlled
/// vectors documented in the module trust model
/// (`floor-capture-hardening-round-3` item 1):
///
/// - `build.rustflags` / `build.rustdocflags` are forced empty, so a committed
///   lint-level flip (`-A warnings`, `--cap-lints allow`) or an extra `--cfg`
///   cannot suppress or steer the diagnostics the floor reads.
/// - `build.rustc-wrapper` / `build.rustc-workspace-wrapper` are forced empty
///   (cargo treats an empty wrapper as "none"), so a compiler wrapper that
///   filters or fabricates diagnostics cannot sit between cargo and rustc.
///
/// These are emitted as argv tokens (`--config`, `KEY=VALUE`) on the real cargo
/// invocation; because the invocation is built as argv and executed directly
/// (never through `sh -c`), the values are passed byte-for-byte with no shell
/// re-splitting. `build.rustc` itself is left to the toolchain default — the
/// floor cannot know the true rustc path, and `env_clear()` already drops any
/// inherited `RUSTC`; a repo `build.rustc` redirect that pointed at a fake
/// compiler would fail to actually build the crate, so the capture fails closed
/// rather than passing vacuously.
const SANITIZING_CONFIG: &[&str] = &[
    "build.rustflags=[]",
    "build.rustdocflags=[]",
    r#"build.rustc-wrapper="""#,
    r#"build.rustc-workspace-wrapper="""#,
];

/// Expand [`SANITIZING_CONFIG`] into the flat `--config KEY=VALUE …` argv the
/// real cargo invocation carries as **global** flags (before the subcommand).
fn sanitizing_config_args() -> Vec<String> {
    SANITIZING_CONFIG
        .iter()
        .flat_map(|kv| ["--config".to_string(), (*kv).to_string()])
        .collect()
}

/// A floor cargo invocation built as **argv** — the supervisor-resolved program
/// plus its already-split arguments — never a shell string
/// (`floor-capture-hardening-round-3` item 1). Executed via
/// [`isolated_command`]`.args(&inv.args)`, so no `sh -c` re-splits, re-quotes, or
/// expands any token.
struct CargoInvocation {
    /// The resolved program to exec (cargo, cargo-clippy, or a fixture script).
    program: String,
    /// The full argument vector, in order.
    args: Vec<String>,
}

impl CargoInvocation {
    /// A shell-ish rendering (`program arg arg …`) for diagnostics only — never
    /// re-parsed or executed. The actual invocation is argv, so this string's
    /// quoting is immaterial.
    fn display(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

/// How the floor executes one capture command.
///
/// The floor's own production captures are always `cargo …`, and those take the
/// [`CaptureExec::Cargo`] path: **argv**, supervisor-resolved, sanitized (item 1).
/// A base command whose first token is not literally `cargo` (a unit-test fixture
/// script or an explicit non-cargo `--test-cmd` / `--clippy-cmd` override) falls
/// back to [`CaptureExec::Shell`] — the composed string run via `sh -c` — because
/// such commands may rely on shell quoting/expansion and the floor cannot
/// meaningfully sanitize cargo config for a non-cargo program anyway.
enum CaptureExec {
    /// Real cargo, run as sanitized argv.
    Cargo(CargoInvocation),
    /// A non-cargo fixture/override, run via `sh -c` (no cargo sanitization).
    Shell(String),
}

impl CaptureExec {
    /// A rendering of the command for diagnostics only.
    fn display(&self) -> String {
        match self {
            CaptureExec::Cargo(inv) => inv.display(),
            CaptureExec::Shell(s) => s.clone(),
        }
    }

    /// The isolated [`Command`] to run (`target_dir` pins `CARGO_TARGET_DIR`).
    fn command(&self, target_dir: &Path) -> Command {
        match self {
            CaptureExec::Cargo(inv) => {
                let mut cmd = isolated_command(&inv.program, Some(target_dir));
                cmd.args(&inv.args);
                cmd
            }
            CaptureExec::Shell(s) => {
                let mut cmd = isolated_command("sh", Some(target_dir));
                cmd.arg("-c").arg(s);
                cmd
            }
        }
    }
}

/// Choose the execution strategy for a capture command (see [`CaptureExec`]).
/// Real cargo (`cargo …`) is built as sanitized argv; anything else is composed
/// into an `sh -c` string with the floor's `floor_flags` appended (the
/// `CARGO_TARGET_DIR` env still pins the target dir for it via
/// [`isolated_command`]).
fn build_capture_exec(base_cmd: &str, target_dir: &Path, floor_flags: &[&str]) -> CaptureExec {
    if base_cmd.split_whitespace().next() == Some("cargo") {
        CaptureExec::Cargo(build_cargo_invocation(base_cmd, target_dir, floor_flags))
    } else {
        let mut parts = vec![base_cmd.to_string()];
        parts.extend(floor_flags.iter().map(ToString::to_string));
        CaptureExec::Shell(parts.join(" "))
    }
}

/// Build a floor cargo invocation as argv from a base command string, the
/// floor-pinned `target_dir`, and the floor's own `floor_flags`
/// (`--no-run`/`--message-format=json`/`--doc`, …).
///
/// The base command is tokenized on whitespace (the floor's own commands never
/// carry quoted, space-containing args — that is the capture contract). The
/// pipeline is:
///
/// 1. **Resolve the program.** A leading `cargo` token becomes [`cargo_bin`]
///    (supervisor-resolved); any other leading token (a fixture script / explicit
///    path) is kept verbatim.
/// 2. **Bypass a `clippy` alias.** When the resolved program is real cargo and
///    the first subcommand token is `clippy`, the invocation is rewritten to the
///    external `cargo-clippy` binary with the `clippy` token dropped, so a repo
///    `[alias] clippy = …` redirect (which shadows the *subcommand*, not the
///    external binary) cannot steer the floor's clippy capture to a benign
///    zero-warning command. `test` is a built-in subcommand and cannot be
///    aliased, so it needs no such rewrite.
/// 3. **Strip repo-supplied target-dir / message-format** from the cargo-side
///    tokens (before any `--`), so only the floor's forced copies survive — an
///    in-command `--target-dir` cannot re-point the floor's pinned cache.
/// 4. **Assemble** the argv in order: `program`, then the sanitizing `--config`
///    globals, then the cargo-side tokens, then `--target-dir <dir>`, then the
///    floor flags, then any `--` and test-binary tokens. The `--target-dir` is
///    always emitted (argv needs no shell-safety dance — the round-2
///    `target_dir_flags` whitespace hack is gone), belt-and-suspenders with the
///    `CARGO_TARGET_DIR` env [`isolated_command`] also sets.
fn build_cargo_invocation(
    base_cmd: &str,
    target_dir: &Path,
    floor_flags: &[&str],
) -> CargoInvocation {
    // cargo flags that take a following value token; an occurrence before `--` is
    // dropped along with its value so only the floor's injected copy survives.
    const DROP_WITH_VALUE: &[&str] = &["--message-format", "--target-dir"];

    let mut tokens = base_cmd.split_whitespace();
    let first = tokens.next().unwrap_or("cargo");
    // A leading literal `cargo` token means the floor drives real cargo: resolve
    // the supervisor cargo and apply the cargo-specific sanitization (alias bypass
    // + `--config` overrides). Any other leading token is a fixture script or an
    // explicit non-cargo program, honoured verbatim with no cargo rewriting (its
    // args must not be prefixed with cargo globals it would reject).
    let is_real_cargo = first == "cargo";
    let mut program = if is_real_cargo {
        cargo_bin()
    } else {
        first.to_string()
    };

    // Partition the remaining tokens at the `--` separator.
    let rest: Vec<&str> = tokens.collect();
    let sep = rest.iter().position(|t| *t == "--");
    let (cargo_side_raw, after_sep): (&[&str], &[&str]) = match sep {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (&rest[..], &[]),
    };

    // Bypass a `clippy` alias: rewrite `cargo clippy …` → `cargo-clippy …`. Only
    // when the program is real cargo (a fixture/explicit path is left alone).
    let mut cargo_side: Vec<&str> = cargo_side_raw.to_vec();
    if is_real_cargo && cargo_side.first() == Some(&"clippy") {
        program = "cargo-clippy".to_string();
        cargo_side.remove(0);
    }

    // Strip any repo-supplied target-dir / message-format from the cargo side.
    let mut kept: Vec<String> = Vec::new();
    let mut skip_next = false;
    for tok in cargo_side {
        if skip_next {
            skip_next = false;
            continue;
        }
        if DROP_WITH_VALUE.contains(&tok) {
            skip_next = true;
            continue;
        }
        if DROP_WITH_VALUE
            .iter()
            .any(|f| tok.strip_prefix(f).is_some_and(|r| r.starts_with('=')))
        {
            continue;
        }
        kept.push(tok.to_string());
    }

    // Assemble: [subcommand …] must precede the floor's cargo-side flags, and the
    // sanitizing `--config` globals precede the subcommand. cargo-clippy forwards
    // globals to `cargo check`, so the same layout is valid there. The `--config`
    // globals are cargo-only: a non-cargo fixture would reject a leading `--config`
    // as an unknown option, so they are omitted there (the invocation is not a real
    // cargo capture anyway).
    let mut args: Vec<String> = Vec::new();
    if is_real_cargo {
        args.extend(sanitizing_config_args());
    }
    args.extend(kept);
    args.push("--target-dir".to_string());
    args.push(target_dir.to_string_lossy().into_owned());
    args.extend(floor_flags.iter().map(ToString::to_string));
    args.extend(after_sep.iter().map(ToString::to_string));

    CargoInvocation { program, args }
}

/// The active `rustc -V` string (isolated), or `"unknown"` if it cannot be
/// determined — recorded as baseline provenance so a snapshot captured under a
/// different toolchain than the tip is detectable.
#[must_use]
pub fn rustc_version(cwd: &Path) -> String {
    isolated_command("rustc", None)
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
/// `target_dir` is the floor-controlled `CARGO_TARGET_DIR` for this capture
/// (`floor-capture-hardening-round-2` item 1 / F4): a fresh, per-snapshot dir the
/// caller pins so baseline and tip never share a warm cache (see
/// [`isolated_command`] / [`build_cargo_invocation`]).
///
/// The invocation is built as **argv** and executed directly
/// (`floor-capture-hardening-round-3` item 1): a supervisor-resolved cargo, a
/// `clippy`-alias bypass (n/a for `test`, which is built-in), and the sanitizing
/// `--config` overrides that neutralize repo `rustflags` / `rustc-wrapper`
/// vectors — never a `sh -c` string.
///
/// Doctests (run by rustdoc, not a `compiler-artifact` binary) are captured in a
/// separate `--doc` pass; this function covers only the `compiler-artifact` test
/// harnesses.
pub fn capture_test_snapshot(
    test_cmd: &str,
    cwd: &Path,
    target_dir: &Path,
) -> Result<TestSnapshot, FloorError> {
    let exec = build_capture_exec(test_cmd, target_dir, &["--no-run", "--message-format=json"]);
    let enumerate_cmd = exec.display();
    let out = exec
        .command(target_dir)
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
    // Record the enumerated target set (F7): the superset gate must see every
    // harness cargo built, independent of how many tests each runs (a target with
    // zero tests still counts — its disappearance at the tip is the shrink we fail
    // closed on).
    let mut snap = TestSnapshot {
        targets: binaries
            .iter()
            .map(|b| format!("{}/{}/{}", b.package, b.target_kind, b.target))
            .collect(),
        ..Default::default()
    };
    for bin in &binaries {
        run_one_test_binary(bin, cwd, target_dir, &mut snap)?;
    }
    Ok(snap)
}

/// Run one enumerated test binary, reconcile it, and fold its target-qualified
/// ids into `snap`. Any inconsistency is a fail-closed [`FloorError`].
fn run_one_test_binary(
    bin: &parse::TestBinary,
    cwd: &Path,
    target_dir: &Path,
    snap: &mut TestSnapshot,
) -> Result<(), FloorError> {
    let out = isolated_command(&bin.executable, Some(target_dir))
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

/// Capture doctests into `snap` — a separate `cargo test -p <pkg> --doc` pass per
/// package with a library target (`floor-capture-hardening-round-3` item 4 / F6).
///
/// Doctests run via rustdoc, not a `compiler-artifact` binary, so they are
/// invisible to [`capture_test_snapshot`]: a new failing doctest, or a test moved
/// *into* a doctest, would otherwise never be observed. Each doctest becomes a
/// `target_kind = "doctest"` [`super::snapshot::TestId`] and each package's lib
/// contributes a `<pkg>/doctest/<lib>` entry to
/// [`super::snapshot::TestSnapshot::targets`], so the existing regression /
/// gaming / enumeration-superset gates cover doctests too. The pass is symmetric
/// across baseline and tip, so it closes a gaming hole rather than adding a
/// crash surface.
///
/// Stable rustdoc emits doctest results as libtest **text** (not JSON), so the
/// output is parsed and reconciled with the same [`reconcile_single_binary`]
/// discipline as a per-binary run: exactly one summary, parsed counts matching
/// the announced summary, and an exit code consistent with it. A compile error in
/// a doctest prints no summary → fail closed. Runs with default features (a
/// documented limitation: a feature-gated doctest may be uncaptured, but the pass
/// is symmetric so it is not a gaming hole for the captured set).
pub fn capture_doctests(
    cwd: &Path,
    target_dir: &Path,
    meta: &super::metadata::WorkspaceMetadata,
    snap: &mut TestSnapshot,
) -> Result<(), FloorError> {
    for pkg in &meta.packages {
        let Some(lib) = super::metadata::lib_target_name(pkg) else {
            continue; // no library target → rustdoc has no doctests to run.
        };
        let exec = build_capture_exec(
            &format!("cargo test -p {} --doc", pkg.name),
            target_dir,
            &[],
        );
        let cmd = exec.display();
        let out = exec
            .command(target_dir)
            .current_dir(cwd)
            .output()
            .map_err(|e| FloorError::Capture {
                what: "tests",
                message: format!("could not run doctests via `{cmd}`: {e}"),
            })?;

        let combined = join_streams(&out.stdout, &out.stderr);
        let report = parse_libtest_report(&combined);
        let summary = reconcile_single_binary(&report).map_err(|d| FloorError::Capture {
            what: "tests",
            message: format!(
                "doctests for package {} ({}): untrustworthy output: {d}. stderr: {}",
                pkg.name,
                lib,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        })?;

        // rustdoc/libtest exits 0 iff nothing failed; a disagreeing exit code is
        // an anomaly (a doctest compile error, a tampered exit) → fail closed.
        let exit_ok = out.status.code() == Some(0);
        if exit_ok != (summary.failed == 0) {
            return Err(FloorError::Capture {
                what: "tests",
                message: format!(
                    "doctests for package {} exit code {:?} inconsistent with summary ({} failed); failing closed",
                    pkg.name,
                    out.status.code(),
                    summary.failed
                ),
            });
        }

        // Every package with a lib contributes a doctest target, even at zero
        // doctests, so its disappearance at the tip is caught by the superset gate.
        snap.targets.insert(format!("{}/doctest/{}", pkg.name, lib));
        snap.passed
            .extend(parse::qualify(&pkg.name, "doctest", &lib, &report.passed));
        snap.failed
            .extend(parse::qualify(&pkg.name, "doctest", &lib, &report.failed));
        snap.ignored
            .extend(parse::qualify(&pkg.name, "doctest", &lib, &report.ignored));
    }
    Ok(())
}

/// Capture a structured [`ClippySnapshot`] from `cargo clippy`
/// `--message-format=json`. Warnings are read from the JSON records keyed by
/// lint code — a `println!`/`build.rs` cannot fabricate one.
///
/// The invocation is built as **argv** and executed directly
/// (`floor-capture-hardening-round-3` item 1): a supervisor-resolved cargo, a
/// `clippy`-alias bypass (`cargo clippy` → the external `cargo-clippy` binary, so
/// a repo `[alias] clippy = …` redirect cannot steer the capture), and the
/// sanitizing `--config` overrides that neutralize repo `rustflags` /
/// `rustc-wrapper` vectors — never a `sh -c` string.
///
/// Fail-closed: the stream must parse, carry a terminal `build-finished`, be
/// free of `error`-level diagnostics, and the process must have exited 0. A
/// clippy run over compilable code exits 0 and emits its warnings at
/// `level: "warning"`; a diagnostic promoted to `error` (a real compile error,
/// or a `deny`-level lint from `[lints]`/`#![deny]`/`-D`) means the code is not
/// in a clean, gateable state, so the capture is rejected rather than trusting a
/// partial warning set.
pub fn capture_clippy_snapshot(
    clippy_cmd: &str,
    cwd: &Path,
    target_dir: &Path,
) -> Result<ClippySnapshot, FloorError> {
    let exec = build_capture_exec(clippy_cmd, target_dir, &["--message-format=json"]);
    let cmd = exec.display();
    let out = exec
        .command(target_dir)
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
        let out = isolated_command("sh", None)
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
    fn build_cargo_invocation_resolves_cargo_and_injects_sanitizing_config() {
        // A leading `cargo` token resolves to the supervisor cargo, the sanitizing
        // `--config` globals precede the subcommand, and the floor's flags follow
        // the subcommand + forced `--target-dir` (item 1).
        let inv = build_cargo_invocation(
            "cargo test --workspace",
            Path::new("/tmp/floor-td"),
            &["--no-run", "--message-format=json"],
        );
        assert_eq!(inv.program, cargo_bin());
        // Sanitizing config is present, and every SANITIZING_CONFIG key appears.
        for kv in SANITIZING_CONFIG {
            let i = inv
                .args
                .iter()
                .position(|a| a == kv)
                .expect("config present");
            assert_eq!(inv.args[i - 1], "--config");
        }
        // Subcommand precedes the floor's cargo-side flags.
        let sub = inv.args.iter().position(|a| a == "test").unwrap();
        let td = inv.args.iter().position(|a| a == "--target-dir").unwrap();
        let ws = inv.args.iter().position(|a| a == "--workspace").unwrap();
        assert!(sub < ws && ws < td, "{:?}", inv.args);
        assert_eq!(inv.args[td + 1], "/tmp/floor-td");
        assert!(inv
            .args
            .ends_with(&["--no-run".to_string(), "--message-format=json".to_string()]));
    }

    #[test]
    fn build_cargo_invocation_bypasses_clippy_alias() {
        // `cargo clippy` is rewritten to the external `cargo-clippy` binary and the
        // `clippy` token dropped, so a repo `[alias] clippy = …` redirect (which
        // shadows the subcommand, not the external binary) cannot steer the capture.
        let inv = build_cargo_invocation(
            "cargo clippy --workspace",
            Path::new("/tmp/td"),
            &["--message-format=json"],
        );
        assert_eq!(inv.program, "cargo-clippy");
        assert!(!inv.args.iter().any(|a| a == "clippy"), "{:?}", inv.args);
        assert!(inv.args.iter().any(|a| a == "--workspace"));
        assert!(inv.args.iter().any(|a| a == "build.rustflags=[]"));
    }

    #[test]
    fn build_cargo_invocation_strips_repo_target_dir_and_message_format() {
        // An in-command `--target-dir` / `--message-format` (space or `=` form) is
        // dropped so only the floor's forced copies survive; args after `--` are
        // preserved for the test binary.
        let inv = build_cargo_invocation(
            "cargo test --target-dir /evil --message-format=short --workspace -- --nocapture",
            Path::new("/tmp/floor-td"),
            &["--no-run", "--message-format=json"],
        );
        assert!(!inv.args.iter().any(|a| a == "/evil"), "{:?}", inv.args);
        assert!(!inv.args.iter().any(|a| a == "--message-format=short"));
        // Exactly one --target-dir, pointing at the floor's dir.
        let tds: Vec<usize> = inv
            .args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--target-dir")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(tds.len(), 1);
        assert_eq!(inv.args[tds[0] + 1], "/tmp/floor-td");
        // Everything after `--` is preserved, after the floor flags.
        assert!(inv
            .args
            .ends_with(&["--".to_string(), "--nocapture".to_string()]));
        let sep = inv.args.iter().position(|a| a == "--").unwrap();
        let nr = inv.args.iter().position(|a| a == "--no-run").unwrap();
        assert!(nr < sep, "floor flags must precede `--`: {:?}", inv.args);
    }

    #[test]
    fn build_cargo_invocation_honours_non_cargo_program_verbatim() {
        // A fixture script / explicit path (not the literal `cargo` token) is kept
        // as-is and never rewritten — even when its first arg is `clippy`.
        let inv = build_cargo_invocation(
            "/tmp/fake-cargo clippy",
            Path::new("/tmp/td"),
            &["--message-format=json"],
        );
        assert_eq!(inv.program, "/tmp/fake-cargo");
        assert!(inv.args.iter().any(|a| a == "clippy"), "{:?}", inv.args);
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
        let snap = capture_clippy_snapshot(&script, dir.path(), dir.path()).unwrap();
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
        let err = capture_clippy_snapshot(&script, dir.path(), dir.path()).unwrap_err();
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
        assert!(capture_clippy_snapshot(&script, dir.path(), dir.path()).is_err());
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
        let snap = capture_test_snapshot(&cargo, dir.path(), dir.path()).unwrap();
        assert_eq!(snap.passed.len(), 1);
        let id = snap.passed.iter().next().unwrap();
        assert_eq!(id.package, "octl-cli");
        assert_eq!(id.target_kind, "lib");
        assert_eq!(id.name, "mymod::works");
        // F7: the enumerated target set is recorded (one canonical key per built
        // harness), independent of how many tests it ran.
        assert_eq!(
            snap.targets.iter().cloned().collect::<Vec<_>>(),
            vec!["octl-cli/lib/octl-cli".to_string()]
        );
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
        let err = capture_test_snapshot(&cargo, dir.path(), dir.path()).unwrap_err();
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
        let err = capture_test_snapshot(&cargo, dir.path(), dir.path()).unwrap_err();
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
        assert!(capture_test_snapshot(&cargo, dir.path(), dir.path()).is_err());
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
        assert!(capture_test_snapshot(&cargo, dir.path(), dir.path()).is_err());
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

    // --- F4: floor-controlled CARGO_TARGET_DIR ---

    #[test]
    fn capture_pins_caller_target_dir_via_env() {
        // The floor pins CARGO_TARGET_DIR to the dir it chose, so an in-repo
        // `build.target-dir` cannot re-point the cache (env beats config). A fake
        // clippy records the CARGO_TARGET_DIR it actually saw; assert it is the
        // dir the floor passed, not the (unset/default) one.
        let dir = TempDir::new().unwrap();
        let td = dir.path().join("floor-target");
        std::fs::create_dir_all(&td).unwrap();
        let sentinel = dir.path().join("seen-target-dir");
        let script = write_script(
            dir.path(),
            "fakeclippy",
            &format!(
                "#!/bin/sh\nprintf '%s' \"$CARGO_TARGET_DIR\" > '{}'\nprintf '{{\"reason\":\"build-finished\",\"success\":true}}\\n'\n",
                sentinel.display()
            ),
        );
        let snap = capture_clippy_snapshot(&script, dir.path(), &td).unwrap();
        assert!(snap.warnings.is_empty());
        let seen = std::fs::read_to_string(&sentinel).unwrap();
        assert_eq!(
            seen,
            td.to_string_lossy(),
            "capture must pin CARGO_TARGET_DIR to the floor's dir"
        );
    }

    #[test]
    fn distinct_target_dirs_defeat_warm_cache_sharing() {
        // done-criteria (c): baseline and tip must NOT share a warm cache, or a
        // warm `cargo clippy` re-emits zero warnings and no-new-clippy passes
        // vacuously. The pipeline gives each capture a fresh dir; here we prove
        // the capture faithfully uses whichever dir it is handed, so two
        // different dirs yield two different recorded target dirs (no sharing).
        let dir = TempDir::new().unwrap();
        let sentinel = dir.path().join("targets.log");
        let script = write_script(
            dir.path(),
            "fakeclippy",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$CARGO_TARGET_DIR\" >> '{}'\nprintf '{{\"reason\":\"build-finished\",\"success\":true}}\\n'\n",
                sentinel.display()
            ),
        );
        let base_td = dir.path().join("base-target");
        let tip_td = dir.path().join("tip-target");
        capture_clippy_snapshot(&script, dir.path(), &base_td).unwrap();
        capture_clippy_snapshot(&script, dir.path(), &tip_td).unwrap();
        let log = std::fs::read_to_string(&sentinel).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_ne!(lines[0], lines[1], "baseline and tip shared a target dir");
    }

    #[test]
    fn build_cargo_invocation_emits_whitespace_target_dir_verbatim() {
        // argv carries the path as ONE token regardless of whitespace/metachars —
        // the round-2 `sh -c` shell-safety dance (which *dropped* the flag on an
        // unsafe path) is gone. A path with a space is a single, exact arg.
        let inv = build_cargo_invocation(
            "cargo test",
            Path::new("/tmp/has space/floor-td"),
            &["--no-run"],
        );
        let td = inv.args.iter().position(|a| a == "--target-dir").unwrap();
        assert_eq!(inv.args[td + 1], "/tmp/has space/floor-td");
    }
}
