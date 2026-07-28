//! Pure parsers for the deterministic floor's raw inputs (design.md §4).
//!
//! These turn tool output — cargo's `--message-format=json` NDJSON stream,
//! libtest's per-test text, and source text — into the floor's value model with
//! **no I/O**. The impure "run the command / read the file" wrappers live in
//! [`super::runner`]. Split out so every parser is a pure function of a `&str`,
//! exhaustively testable from captured fixtures without shelling out or
//! depending on a toolchain version.
//!
//! # Injection resistance (`floor-capture-trust-model`)
//!
//! Clippy warnings are read from cargo's structured JSON records (lint code +
//! package + span file), not from a text line, so a `println!`/`build.rs`
//! cannot fabricate one. libtest still emits per-test *text* on stable (its JSON
//! format is nightly-only), so the test path defends differently:
//! [`parse_libtest_report`] parses both the per-test lines **and** libtest's own
//! `test result:` summary, and [`reconcile_single_binary`] rejects the capture
//! if a forged `test x ... ok` line makes the parsed count disagree with the
//! authoritative announced count, or if a forged `test result:` line makes a
//! single binary announce more than one summary.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::snapshot::{ClippyWarning, TestId};

// ---------------------------------------------------------------------------
// cargo --message-format=json (NDJSON compiler messages / artifacts)
// ---------------------------------------------------------------------------

/// One line of cargo's `--message-format=json` stream. Only the fields the
/// floor needs are modelled; cargo's many other keys are ignored by serde.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum CargoMessage {
    /// A compiler/clippy diagnostic (warning or error).
    CompilerMessage {
        /// Cargo package id (e.g. `path+file:///…#octl-cli@0.1.0`).
        #[serde(default)]
        package_id: String,
        /// The diagnostic payload.
        message: CargoDiagnostic,
    },
    /// A built artifact — for test binaries this carries the executable path.
    CompilerArtifact {
        /// Cargo package id.
        #[serde(default)]
        package_id: String,
        /// The target that was built.
        target: CargoTarget,
        /// Build profile flags.
        #[serde(default)]
        profile: CargoProfile,
        /// The produced executable, if any.
        #[serde(default)]
        executable: Option<String>,
    },
    /// The terminal record of a cargo invocation.
    BuildFinished {
        /// Whether the build succeeded.
        success: bool,
    },
    /// Any other reason (`build-script-executed`, …) — ignored.
    #[serde(other)]
    Other,
}

/// A cargo build target (`{ "name": "octl-cli", "kind": ["lib"] }`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CargoTarget {
    /// Target name (crate name for a lib, file stem for an integration test).
    #[serde(default)]
    pub name: String,
    /// Target kinds (`["lib"]`, `["test"]`, `["bin"]`, …).
    #[serde(default)]
    pub kind: Vec<String>,
}

impl CargoTarget {
    /// The first (primary) kind, or `"unknown"` when cargo omitted it.
    #[must_use]
    pub fn primary_kind(&self) -> &str {
        self.kind.first().map_or("unknown", String::as_str)
    }
}

/// The subset of a cargo build profile the floor reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CargoProfile {
    /// True when the artifact is a test harness binary.
    #[serde(default)]
    pub test: bool,
}

/// A compiler diagnostic payload inside a `compiler-message`.
#[derive(Debug, Clone, Deserialize)]
pub struct CargoDiagnostic {
    /// `"warning"`, `"error"`, `"note"`, …
    #[serde(default)]
    pub level: String,
    /// The short (non-rendered) message.
    #[serde(default)]
    pub message: String,
    /// The lint code (`{ "code": "clippy::needless_return" }`), if any.
    #[serde(default)]
    pub code: Option<CargoCode>,
    /// Diagnostic spans; the primary one names the offending file.
    #[serde(default)]
    pub spans: Vec<CargoSpan>,
}

/// A lint code wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct CargoCode {
    /// The lint identifier (`clippy::needless_return`, `unused_variables`).
    #[serde(default)]
    pub code: String,
}

/// A single diagnostic span.
#[derive(Debug, Clone, Deserialize)]
pub struct CargoSpan {
    /// Source file the span points at (repo-relative as cargo reports it).
    #[serde(default)]
    pub file_name: String,
    /// Whether this is the primary span of the diagnostic.
    #[serde(default)]
    pub is_primary: bool,
}

/// Why a cargo NDJSON stream could not be trusted. The floor treats any of
/// these as fail-closed (an incomplete/forged capture must not read as green).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CargoStreamError {
    /// A non-blank stdout line was not valid cargo JSON — the stream is partial
    /// or was polluted with non-JSON text.
    UnparseableLine {
        /// 1-based line number.
        line_no: usize,
        /// A short prefix of the offending line, for diagnosis.
        snippet: String,
    },
}

impl std::fmt::Display for CargoStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CargoStreamError::UnparseableLine { line_no, snippet } => {
                write!(f, "unparseable cargo JSON at line {line_no}: {snippet:?}")
            }
        }
    }
}

/// Parse a cargo `--message-format=json` stdout stream (NDJSON) into typed
/// messages. **Every** non-blank line must be valid cargo JSON; the first line
/// that is not fails the whole parse (`floor-capture-trust-model`: reject on
/// unparseable/partial output rather than skipping unrecognized lines). With
/// `--message-format=json`, cargo emits only JSON objects on stdout and nothing
/// runs during a compile/`--no-run`, so any non-JSON line is anomalous.
pub fn parse_cargo_stream(stdout: &str) -> Result<Vec<CargoMessage>, CargoStreamError> {
    let mut out = Vec::new();
    for (idx, raw) in stdout.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<CargoMessage>(line) else {
            let snippet: String = line.chars().take(80).collect();
            return Err(CargoStreamError::UnparseableLine {
                line_no: idx + 1,
                snippet,
            });
        };
        out.push(msg);
    }
    Ok(out)
}

/// True if any message is an `error`-level compiler diagnostic — the code did
/// not compile, so the capture must fail closed (no vacuous empty snapshot).
#[must_use]
pub fn has_compile_error(messages: &[CargoMessage]) -> bool {
    messages.iter().any(
        |m| matches!(m, CargoMessage::CompilerMessage { message, .. } if message.level == "error"),
    )
}

/// The success flag of the terminal `build-finished` record, or `None` if the
/// stream carried none (truncated / never ran → fail closed).
#[must_use]
pub fn build_finished(messages: &[CargoMessage]) -> Option<bool> {
    messages.iter().rev().find_map(|m| match m {
        CargoMessage::BuildFinished { success } => Some(*success),
        _ => None,
    })
}

/// Extract the structured clippy warnings from a parsed cargo stream.
///
/// Only `warning`-level diagnostics **with a lint code** are collected: a
/// code-less warning (a `build.rs` `cargo:warning=…`, which is repo-controlled
/// and injectable) is not a clippy lint and is dropped. The identity keys on
/// `(lint, package, primary-span file, message)` — never the `line:col` span —
/// so a line-shifting refactor does not flip an unchanged warning to "new".
#[must_use]
pub fn clippy_warnings(messages: &[CargoMessage]) -> BTreeSet<ClippyWarning> {
    let mut set = BTreeSet::new();
    for m in messages {
        let CargoMessage::CompilerMessage {
            package_id,
            message,
        } = m
        else {
            continue;
        };
        if message.level != "warning" {
            continue;
        }
        let Some(code) = message.code.as_ref().map(|c| c.code.clone()) else {
            continue;
        };
        if code.is_empty() {
            continue;
        }
        let file = message
            .spans
            .iter()
            .find(|s| s.is_primary)
            .or_else(|| message.spans.first())
            .map(|s| s.file_name.clone())
            .unwrap_or_default();
        set.insert(ClippyWarning {
            lint: code,
            package: short_package_name(package_id),
            file,
            message: message.message.clone(),
        });
    }
    set
}

/// A test-harness binary cargo built, with the metadata needed to
/// target-qualify the tests it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestBinary {
    /// Short package name.
    pub package: String,
    /// Target kind (`lib`, `test`, `bin`, …).
    pub target_kind: String,
    /// Target name.
    pub target: String,
    /// Path to the executable to run.
    pub executable: String,
}

/// Extract the test-harness executables from a parsed cargo stream — the
/// `compiler-artifact` records whose profile is a test build and which produced
/// an executable. Each becomes a target-qualification context for the libtest
/// output it emits.
#[must_use]
pub fn test_binaries(messages: &[CargoMessage]) -> Vec<TestBinary> {
    messages
        .iter()
        .filter_map(|m| match m {
            CargoMessage::CompilerArtifact {
                package_id,
                target,
                profile,
                executable: Some(exe),
            } if profile.test => Some(TestBinary {
                package: short_package_name(package_id),
                target_kind: target.primary_kind().to_string(),
                target: target.name.clone(),
                executable: exe.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Reduce a cargo `package_id` to a short package name across cargo id formats:
/// - modern `path+file:///abs#name@1.2.3` or `registry+…#name@1.2.3`
///   (also the `#name` shorthand when the dir stem equals the name);
/// - legacy `name 1.2.3 (source)`.
///
/// Best-effort and only used for a human-facing/audit label + as part of the
/// warning identity; the lint code carries the security-relevant identity.
#[must_use]
pub fn short_package_name(package_id: &str) -> String {
    if let Some((_, after)) = package_id.rsplit_once('#') {
        // `name@version` or bare `name`.
        let name = after.split('@').next().unwrap_or(after);
        if !name.is_empty() {
            return name.to_string();
        }
    }
    // Legacy `name version (source)`.
    package_id
        .split_whitespace()
        .next()
        .unwrap_or(package_id)
        .to_string()
}

// ---------------------------------------------------------------------------
// libtest text (per-test lines + the authoritative summary)
// ---------------------------------------------------------------------------

/// libtest's `test result:` summary counts — the authoritative tally the
/// harness itself computed. Parsed counts are reconciled against these so a
/// forged `test … ok` line cannot inflate the pass set undetected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibtestSummary {
    /// Announced passing count.
    pub passed: usize,
    /// Announced failing count.
    pub failed: usize,
    /// Announced ignored count.
    pub ignored: usize,
    /// Announced filtered-out count. The floor runs each binary directly with no
    /// filter, so a non-zero value means a filter leaked in (env/config/wrapper)
    /// and the capture is only a *subset* — reconciliation rejects it.
    pub filtered_out: usize,
}

/// The parsed result of running **one** libtest binary: the per-test names by
/// outcome plus every `test result:` summary line seen (a well-formed single
/// binary prints exactly one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibtestReport {
    /// Names of tests reported `ok`.
    pub passed: Vec<String>,
    /// Names of tests reported `FAILED`.
    pub failed: Vec<String>,
    /// Names of tests reported `ignored`.
    pub ignored: Vec<String>,
    /// Every `test result:` summary line parsed (should be exactly one).
    pub summaries: Vec<LibtestSummary>,
}

/// Parse the text output of a single libtest binary into a [`LibtestReport`].
///
/// libtest prints one `test <name> ... <outcome>` line per test and one
/// `test result: <status>. <p> passed; <f> failed; <i> ignored; …` summary at
/// the end. Both shapes are parsed; unrecognized lines (compile logs, a
/// `running N tests` header, benchmark lines) are ignored. Injected lines that
/// *do* match a shape are captured here and caught by
/// [`reconcile_single_binary`], not silently trusted.
#[must_use]
pub fn parse_libtest_report(output: &str) -> LibtestReport {
    let mut report = LibtestReport::default();
    for raw in output.lines() {
        let line = raw.trim();
        if let Some(summary) = parse_result_line(line) {
            report.summaries.push(summary);
            continue;
        }
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        let Some((name, outcome)) = rest.split_once(" ... ") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let outcome = outcome.trim();
        if outcome == "ok" || outcome.starts_with("ok ") {
            report.passed.push(name.to_string());
        } else if outcome.starts_with("FAILED") {
            report.failed.push(name.to_string());
        } else if outcome.starts_with("ignored") {
            report.ignored.push(name.to_string());
        }
        // bench lines and unknown outcomes are left out.
    }
    report
}

/// Parse a `test result: ok. 3 passed; 1 failed; 2 ignored; 0 measured; …`
/// line into its counts, or `None` if the line is not a summary.
fn parse_result_line(line: &str) -> Option<LibtestSummary> {
    let rest = line.strip_prefix("test result:")?;
    // Segments look like `3 passed`, `1 failed`, `2 ignored` separated by `;`.
    let mut summary = LibtestSummary::default();
    for seg in rest.split(';') {
        let seg = seg
            .trim()
            .trim_start_matches("ok.")
            .trim_start_matches("FAILED.");
        let mut it = seg.split_whitespace();
        let (Some(num), Some(label)) = (it.next(), it.next()) else {
            continue;
        };
        let Ok(n) = num.parse::<usize>() else {
            continue;
        };
        match label {
            "passed" => summary.passed = n,
            "failed" => summary.failed = n,
            "ignored" => summary.ignored = n,
            // `filtered out` — the label after the count is `filtered`.
            "filtered" => summary.filtered_out = n,
            _ => {}
        }
    }
    Some(summary)
}

/// Why a single libtest binary's output could not be trusted — the fail-closed
/// signal for the test capture path (`floor-capture-trust-model` item 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibtestDiscrepancy {
    /// No `test result:` summary — the binary did not run to completion, or its
    /// output was truncated.
    NoSummary,
    /// More than one `test result:` summary from one binary — a forged summary
    /// line was injected.
    MultipleSummaries {
        /// How many summaries were seen.
        count: usize,
    },
    /// The parsed per-test counts disagree with libtest's announced counts — a
    /// forged `test … <outcome>` line was injected (or output was lost).
    CountMismatch {
        /// libtest's authoritative announced counts.
        announced: LibtestSummary,
        /// `(passed, failed, ignored)` counted from the per-test lines.
        parsed: (usize, usize, usize),
    },
    /// libtest reported a non-zero `filtered out` count — the run only observed
    /// a subset of the binary's tests (a leaked filter), so it cannot be trusted
    /// as the full picture.
    Filtered {
        /// How many tests were filtered out.
        count: usize,
    },
}

impl std::fmt::Display for LibtestDiscrepancy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibtestDiscrepancy::NoSummary => f.write_str("no `test result:` summary line"),
            LibtestDiscrepancy::MultipleSummaries { count } => {
                write!(f, "{count} `test result:` summaries from one binary")
            }
            LibtestDiscrepancy::CountMismatch { announced, parsed } => write!(
                f,
                "count mismatch: announced {}p/{}f/{}i, parsed {}p/{}f/{}i",
                announced.passed, announced.failed, announced.ignored, parsed.0, parsed.1, parsed.2
            ),
            LibtestDiscrepancy::Filtered { count } => {
                write!(f, "{count} test(s) filtered out; capture is only a subset")
            }
        }
    }
}

/// Reconcile one libtest binary's [`LibtestReport`] against its own announced
/// summary. Succeeds only when there is **exactly one** summary and the parsed
/// per-test counts match it exactly. Any injected `test … ok` line inflates the
/// parsed count above the announced count; any injected `test result:` line
/// produces a second summary — both are rejected, so a forged pass never
/// reaches the snapshot.
pub fn reconcile_single_binary(
    report: &LibtestReport,
) -> Result<LibtestSummary, LibtestDiscrepancy> {
    match report.summaries.as_slice() {
        [] => Err(LibtestDiscrepancy::NoSummary),
        [summary] => {
            // A leaked filter means we only saw a subset — reject before
            // trusting the counts.
            if summary.filtered_out > 0 {
                return Err(LibtestDiscrepancy::Filtered {
                    count: summary.filtered_out,
                });
            }
            let parsed = (
                report.passed.len(),
                report.failed.len(),
                report.ignored.len(),
            );
            let announced = (summary.passed, summary.failed, summary.ignored);
            if parsed == announced {
                Ok(*summary)
            } else {
                Err(LibtestDiscrepancy::CountMismatch {
                    announced: *summary,
                    parsed,
                })
            }
        }
        many => Err(LibtestDiscrepancy::MultipleSummaries { count: many.len() }),
    }
}

/// Build the target-qualified [`TestId`]s for one binary from its reconciled
/// report, given the binary's package/kind/target metadata. Applied only after
/// [`reconcile_single_binary`] has proven the report trustworthy.
#[must_use]
pub fn qualify(
    package: &str,
    target_kind: &str,
    target: &str,
    names: &[String],
) -> BTreeSet<TestId> {
    names
        .iter()
        .map(|n| TestId::new(package, target_kind, target, n))
        .collect()
}

// ---------------------------------------------------------------------------
// assertion-density counting (unchanged from T3 — a relative, crude signal)
// ---------------------------------------------------------------------------

/// Count assert-family macro invocations in a source string — the crude
/// "assertion density" signal (design.md §4: "crude counts are fine —
/// `assert*!` occurrences").
///
/// Comments and string/char literals are stripped first
/// ([`strip_comments_and_strings`]) so the count reflects *code*, not padding:
/// an `assert!` inside `// …`, `/* … */`, `"…"`, or `r#"…"#` no longer counts.
/// This closes the trivial "delete a real assertion, add `// assert!()` to hold
/// the number" gaming hole.
///
/// After stripping, counts an identifier immediately followed by `!` when the
/// identifier is `assert` / `debug_assert` or starts with `assert_` /
/// `debug_assert_` (so `assert!`, `assert_eq!`, `assert_ne!`, `assert_matches!`,
/// `debug_assert!`, … all count).
///
/// Still deliberately crude, and NOT a proof of test power: it cannot tell
/// `assert!(true)` from a real assertion, counts per-file rather than per
/// `#[test]`, and the stripper is a lexer, not a full parser (e.g. `assert!
/// /*c*/ (x)` with whitespace/comments before `!` is missed). It is a
/// *relative* regression signal. Hardening this to a semantic, per-test
/// AST measure is deferred to the `floor-capture-trust-model` follow-up.
#[must_use]
pub fn count_assert_macros(src: &str) -> usize {
    let code = strip_comments_and_strings(src);
    let bytes = code.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        // Find the start of an identifier (not preceded by an identifier char).
        if is_ident_start(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &code[start..i];
            if i < bytes.len() && bytes[i] == b'!' && is_assert_macro(ident) {
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    count
}

/// Remove Rust line/block comments and string/char literals from `src`,
/// replacing each stripped region with a single space (so tokens on either side
/// stay separated). A crude lexer — good enough to stop assertion-count padding
/// via comments and string literals, not a full Rust parser.
///
/// Handles: `//` line comments; `/* … */` block comments (nested, as Rust
/// allows); normal `"…"` and byte `b"…"` strings (with `\` escapes); raw
/// `r"…"` / `r#"…"#` / `br#"…"#` strings (matched hash count); and simple char
/// literals (`'a'`, `'\n'`). A lifetime (`'a`, `'static`) is left intact.
#[must_use]
pub fn strip_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    let n = b.len();
    let prev_is_ident = |out: &[u8]| out.last().is_some_and(|&c| is_ident_char(c));
    while i < n {
        // Line comment.
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (nested).
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
            let mut depth = 1u32;
            i += 2;
            while i < n && depth > 0 {
                if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(b' ');
            continue;
        }
        // Raw string: (b?) r #* " … " #*  — only when not part of an identifier.
        if (b[i] == b'r' || (b[i] == b'b' && i + 1 < n && b[i + 1] == b'r')) && !prev_is_ident(&out)
        {
            if let Some(next) = scan_raw_string(b, i) {
                out.push(b' ');
                i = next;
                continue;
            }
        }
        // Normal/byte string.
        if b[i] == b'"' || (b[i] == b'b' && i + 1 < n && b[i + 1] == b'"' && !prev_is_ident(&out)) {
            let mut j = if b[i] == b'"' { i + 1 } else { i + 2 };
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            out.push(b' ');
            i = j;
            continue;
        }
        // Char literal vs lifetime.
        if b[i] == b'\'' {
            if let Some(next) = scan_char_literal(b, i) {
                out.push(b' ');
                i = next;
                continue;
            }
            // Lifetime: keep the quote, fall through to copy the rest.
            out.push(b'\'');
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    // Code regions were valid UTF-8 in the source and were copied verbatim; the
    // only substitutions are single-byte ASCII spaces, so this cannot fail.
    String::from_utf8(out).unwrap_or_default()
}

/// If a raw-string literal starts at `i`, return the index just past its close;
/// else `None`. `i` points at the leading `r` or `b`.
fn scan_raw_string(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    if j >= n || b[j] != b'r' {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while j < n && b[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    if j >= n || b[j] != b'"' {
        return None;
    }
    j += 1;
    // Scan for a closing `"` followed by exactly `hashes` `#`.
    while j < n {
        if b[j] == b'"' {
            let close = j + 1;
            if close + hashes <= n && b[close..close + hashes].iter().all(|&c| c == b'#') {
                return Some(close + hashes);
            }
        }
        j += 1;
    }
    Some(n) // unterminated: consume to end
}

/// If a simple char literal starts at `i` (`'a'`, `'\n'`, `'\''`, `'\\'`),
/// return the index just past its close; else `None` (a lifetime like `'a`).
fn scan_char_literal(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    // i points at the opening `'`.
    let content_end = if i + 1 < n && b[i + 1] == b'\\' {
        i + 3 // `'` `\` <escaped>
    } else {
        i + 2 // `'` <char>
    };
    if content_end < n && b[content_end] == b'\'' {
        Some(content_end + 1)
    } else {
        None
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_assert_macro(ident: &str) -> bool {
    ident == "assert"
        || ident == "debug_assert"
        || ident.starts_with("assert_")
        || ident.starts_with("debug_assert_")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- cargo JSON: clippy warnings ---

    #[test]
    fn parses_clippy_warnings_from_json_keyed_by_lint_and_file() {
        let stream = concat!(
            r#"{"reason":"compiler-message","package_id":"path+file:///x#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"warning","message":"unused variable: `x`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/a.rs","line_start":3,"is_primary":true}]}}"#,
            "\n",
            r#"{"reason":"compiler-message","package_id":"path+file:///x#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"warning","message":"needless return","code":{"code":"clippy::needless_return"},"spans":[{"file_name":"src/b.rs","line_start":10,"is_primary":true}]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":false}"#,
        );
        let msgs = parse_cargo_stream(stream).unwrap();
        assert_eq!(build_finished(&msgs), Some(false));
        assert!(!has_compile_error(&msgs));
        let warnings = clippy_warnings(&msgs);
        assert_eq!(warnings.len(), 2);
        assert!(warnings
            .iter()
            .any(|w| w.lint == "unused_variables" && w.file == "src/a.rs" && w.package == "pkg"));
        assert!(warnings
            .iter()
            .any(|w| w.lint == "clippy::needless_return" && w.file == "src/b.rs"));
    }

    #[test]
    fn clippy_identity_is_stable_across_line_shifts() {
        let mk = |line: u32| {
            format!(
                r#"{{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{{"name":"pkg","kind":["lib"]}},"message":{{"level":"warning","message":"unused variable: `x`","code":{{"code":"unused_variables"}},"spans":[{{"file_name":"src/a.rs","line_start":{line},"is_primary":true}}]}}}}"#
            )
        };
        let a = clippy_warnings(&parse_cargo_stream(&mk(3)).unwrap());
        let b = clippy_warnings(&parse_cargo_stream(&mk(47)).unwrap());
        assert_eq!(a, b, "a line shift must not change warning identity");
    }

    #[test]
    fn code_less_and_error_diagnostics_are_not_clippy_warnings() {
        // A build-script `cargo:warning=` has no lint code ⇒ dropped (injectable).
        // An error-level diagnostic is not a warning ⇒ not collected (but does
        // flip has_compile_error).
        let stream = concat!(
            r#"{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{"name":"build-script-build","kind":["custom-build"]},"message":{"level":"warning","message":"forged","code":null,"spans":[]}}"#,
            "\n",
            r#"{"reason":"compiler-message","package_id":"p#pkg@0.1.0","target":{"name":"pkg","kind":["lib"]},"message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[{"file_name":"src/a.rs","is_primary":true}]}}"#,
        );
        let msgs = parse_cargo_stream(stream).unwrap();
        assert!(clippy_warnings(&msgs).is_empty());
        assert!(has_compile_error(&msgs));
    }

    #[test]
    fn real_cargo_reasons_including_build_script_parse_as_other() {
        // Regression guard: a crate with a `build.rs` emits `build-script-executed`
        // records. `#[serde(other)]` on the internally-tagged enum MUST accept
        // them (as `Other`) — if it rejected them as unparseable, every
        // build-script crate would fail closed. This is a real line from
        // `cargo build --message-format=json`.
        let stream = concat!(
            r#"{"reason":"build-script-executed","package_id":"registry+https://github.com/rust-lang/crates.io-index#libc@0.2.186","linked_libs":[],"linked_paths":[],"cfgs":["freebsd12"],"env":[],"out_dir":"/x/out"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
        );
        let msgs = parse_cargo_stream(stream).expect("build-script-executed must not fail closed");
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], CargoMessage::Other));
        assert_eq!(build_finished(&msgs), Some(true));
    }

    #[test]
    fn unparseable_line_fails_closed() {
        let stream = concat!(
            r#"{"reason":"build-finished","success":true}"#,
            "\n",
            "test injected ... ok\n", // a stray non-JSON line
        );
        let err = parse_cargo_stream(stream).unwrap_err();
        assert!(matches!(
            err,
            CargoStreamError::UnparseableLine { line_no: 2, .. }
        ));
    }

    // --- cargo JSON: test binary enumeration ---

    #[test]
    fn extracts_test_binaries_with_target_metadata() {
        let stream = concat!(
            r#"{"reason":"compiler-artifact","package_id":"p#octl-cli@0.1.0","target":{"name":"octl-cli","kind":["lib"]},"profile":{"test":true},"executable":"/t/deps/octl_cli-abc"}"#,
            "\n",
            r#"{"reason":"compiler-artifact","package_id":"p#octl-cli@0.1.0","target":{"name":"e2e","kind":["test"]},"profile":{"test":true},"executable":"/t/deps/e2e-def"}"#,
            "\n",
            // A non-test artifact (the normal lib build) is ignored.
            r#"{"reason":"compiler-artifact","package_id":"p#octl-cli@0.1.0","target":{"name":"octl-cli","kind":["lib"]},"profile":{"test":false},"executable":null}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
        );
        let msgs = parse_cargo_stream(stream).unwrap();
        let bins = test_binaries(&msgs);
        assert_eq!(bins.len(), 2);
        assert_eq!(bins[0].target_kind, "lib");
        assert_eq!(bins[0].target, "octl-cli");
        assert_eq!(bins[1].target_kind, "test");
        assert_eq!(bins[1].target, "e2e");
        assert_eq!(bins[1].package, "octl-cli");
    }

    #[test]
    fn short_package_name_handles_formats() {
        assert_eq!(
            short_package_name("path+file:///x#octl-cli@0.1.0"),
            "octl-cli"
        );
        assert_eq!(
            short_package_name("registry+https://x#serde@1.0.0"),
            "serde"
        );
        assert_eq!(
            short_package_name("octl-cli 0.1.0 (path+file:///x)"),
            "octl-cli"
        );
        assert_eq!(short_package_name("p#bare"), "bare");
    }

    // --- libtest reconcile (fail-closed) ---

    #[test]
    fn parses_mixed_libtest_outcomes_and_summary() {
        let out = "\
running 5 tests
test export::csv::roundtrip ... ok
test export::csv::escaping ... FAILED
test routes::account::export_ok ... ok
test slow::network ... ignored
test slow::flaky ... ignored, needs network

test result: FAILED. 2 passed; 1 failed; 2 ignored; 0 measured; 0 filtered out
";
        let report = parse_libtest_report(out);
        assert_eq!(report.passed.len(), 2);
        assert_eq!(report.failed, vec!["export::csv::escaping"]);
        assert_eq!(report.ignored.len(), 2);
        let summary = reconcile_single_binary(&report).unwrap();
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.ignored, 2);
    }

    #[test]
    fn forged_ok_line_fails_reconciliation() {
        // The core injection test (done-criteria a): a `println!` emits a fake
        // passing line, but libtest's own summary announces only the real one.
        // Parsed pass-count (2) exceeds announced (1) ⇒ rejected, so the forged
        // test never becomes a passing TestId.
        let out = "\
running 1 test
test real::actual ... ok
test forged::injected ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        let report = parse_libtest_report(out);
        assert_eq!(report.passed.len(), 2, "both lines parse as text");
        let err = reconcile_single_binary(&report).unwrap_err();
        assert_eq!(
            err,
            LibtestDiscrepancy::CountMismatch {
                announced: LibtestSummary {
                    passed: 1,
                    failed: 0,
                    ignored: 0,
                    filtered_out: 0,
                },
                parsed: (2, 0, 0),
            }
        );
    }

    #[test]
    fn filtered_out_run_fails_closed() {
        // A leaked filter (`--skip`, an env/config filter) makes libtest run only
        // a subset; the announced `filtered out` count is non-zero and the
        // capture is rejected rather than trusted as the whole suite.
        let out = "\
running 1 test
test kept::one ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out
";
        let report = parse_libtest_report(out);
        assert_eq!(
            reconcile_single_binary(&report).unwrap_err(),
            LibtestDiscrepancy::Filtered { count: 5 }
        );
    }

    #[test]
    fn forged_summary_line_is_multiple_summaries() {
        let out = "\
test real::actual ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 99 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        let report = parse_libtest_report(out);
        assert_eq!(
            reconcile_single_binary(&report).unwrap_err(),
            LibtestDiscrepancy::MultipleSummaries { count: 2 }
        );
    }

    #[test]
    fn missing_summary_fails_closed() {
        // A truncated run (killed / panicked before the summary) has no summary
        // line ⇒ rejected, never read as an all-pass.
        let out = "test a::b ... ok\n";
        assert_eq!(
            reconcile_single_binary(&parse_libtest_report(out)).unwrap_err(),
            LibtestDiscrepancy::NoSummary
        );
    }

    #[test]
    fn zero_test_binary_reconciles() {
        let out = "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let report = parse_libtest_report(out);
        assert_eq!(reconcile_single_binary(&report).unwrap().passed, 0);
    }

    #[test]
    fn qualify_builds_target_qualified_ids() {
        let ids = qualify("octl-cli", "test", "e2e", &["a::b".to_string()]);
        assert!(ids.contains(&TestId::new("octl-cli", "test", "e2e", "a::b")));
    }

    #[test]
    fn ok_with_trailing_note_is_passed() {
        let report = parse_libtest_report(
            "test a::b ... ok (fast)\ntest result: ok. 1 passed; 0 failed; 0 ignored\n",
        );
        assert_eq!(report.passed, vec!["a::b"]);
    }

    // --- assertion counting (regression-tested from T3) ---

    #[test]
    fn counts_assert_family_macros() {
        let src = r"
            fn t() {
                assert!(x);
                assert_eq!(a, b);
                assert_ne!(a, b);
                debug_assert!(y);
                debug_assert_eq!(a, b);
                assert_matches!(v, Some(_));
            }
        ";
        assert_eq!(count_assert_macros(src), 6);
    }

    #[test]
    fn does_not_count_non_assert_or_bare_idents() {
        let src = "assertion_helper(); let assert = 1; reassert!(z); asserts!(w);";
        assert_eq!(count_assert_macros(src), 0);
    }

    #[test]
    fn counts_at_string_boundaries() {
        assert_eq!(count_assert_macros("assert!(true)"), 1);
        assert_eq!(count_assert_macros("x.assert_eq!(a,b)"), 1);
    }

    #[test]
    fn does_not_count_assertions_in_comments_or_strings() {
        let src = r#"
            fn t() {
                assert!(real);                    // one real assertion
                // assert!(fake); assert_eq!(a,b);
                /* assert!(also_fake);
                   assert_ne!(x, y); */
                let s = "assert!(in_string); assert_eq!(q, r)";
                let raw = r"assert!(in_raw_string)";
                let _c = '"'; // a quote char literal must not desync string parsing
            }
        "#;
        assert_eq!(count_assert_macros(src), 1);
    }

    #[test]
    fn strip_handles_nested_block_comments_and_raw_strings() {
        let stripped = strip_comments_and_strings("a /* x /* y */ z */ b");
        assert!(stripped.contains('a') && stripped.contains('b'));
        assert!(!stripped.contains('x') && !stripped.contains('z'));

        let stripped = strip_comments_and_strings(r##"code r#"in "quotes" here"# more"##);
        assert!(stripped.contains("code") && stripped.contains("more"));
        assert!(!stripped.contains("quotes"));
    }

    #[test]
    fn lifetime_is_not_mistaken_for_a_char_literal() {
        let src = "fn f<'a>(x: &'a T) { assert!(x); }";
        assert_eq!(count_assert_macros(src), 1);
    }
}
