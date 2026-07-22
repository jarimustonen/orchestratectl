//! Pure parsers for the deterministic floor's raw inputs (design.md §4).
//!
//! These turn tool output (libtest, clippy) and source text into the floor's
//! value model with **no I/O** — the impure "run the command / read the file"
//! wrappers live in [`super::runner`]. Split out so every parser is a pure
//! function of a `&str`, exhaustively testable from captured fixtures without
//! shelling out or depending on a toolchain version.

use super::snapshot::{ClippySnapshot, TestSnapshot};

/// Parse libtest's human-readable output into a [`TestSnapshot`].
///
/// libtest prints one line per test, `test <name> ... <outcome>`, where the
/// outcome starts with `ok`, `FAILED`, or `ignored` (an ignore reason may
/// follow, e.g. `ignored, needs network`). The `test result:` summary line is
/// deliberately *not* of that shape and is skipped. Doc-test lines
/// (`test src/lib.rs - foo (line 3) ... ok`) parse the same way, keyed by their
/// printed name.
///
/// This is stable-toolchain text parsing (libtest's JSON format is
/// nightly-only). It is deliberately lenient: an unrecognized line is ignored
/// rather than failing the parse, so a benchmark line or a stray `println!`
/// cannot corrupt the snapshot.
#[must_use]
pub fn parse_libtest_output(output: &str) -> TestSnapshot {
    let mut snap = TestSnapshot::default();
    for raw in output.lines() {
        let line = raw.trim();
        // Must be `test <name> ... <outcome>`, and not the `test result:` tally.
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        if line.starts_with("test result:") {
            continue;
        }
        let Some((name, outcome)) = rest.split_once(" ... ") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let outcome = outcome.trim();
        // Match on the leading token so an ignore-reason suffix or a
        // `FAILED (should panic)` note doesn't defeat classification.
        if outcome == "ok" || outcome.starts_with("ok ") {
            snap.passed.insert(name.to_string());
        } else if outcome.starts_with("FAILED") {
            snap.failed.insert(name.to_string());
        } else if outcome.starts_with("ignored") {
            snap.ignored.insert(name.to_string());
        }
        // Anything else (bench results, unknown outcomes) is left out.
    }
    snap
}

/// Parse `cargo clippy --message-format=short` output into a [`ClippySnapshot`].
///
/// Short format prints each diagnostic on a single line. A warning is any line
/// containing `warning:` — with a `path:line:col:` prefix for a located lint,
/// or a bare `warning:` for a crate-level one. The per-crate tally line (the
/// "... generated N warnings" summary) is excluded so the count
/// itself is not mistaken for a warning. The whole line is the warning's
/// identity: the same lint at the same place prints identically across runs, so
/// set-difference against the baseline yields exactly the *new* warnings.
///
/// Errors (`error:` / `error[E….]:`) are not collected — the floor's clippy
/// gate is specifically "no new *warnings*"; a hard error fails the build (and
/// the relevant check) on its own.
#[must_use]
pub fn parse_clippy_short(output: &str) -> ClippySnapshot {
    let mut snap = ClippySnapshot::default();
    for raw in output.lines() {
        let line = raw.trim();
        if !line.contains("warning:") {
            continue;
        }
        // Drop the summary tally line(s).
        if line.starts_with("warning:") && line.contains("generated") {
            continue;
        }
        snap.warnings.insert(line.to_string());
    }
    snap
}

/// Count assert-family macro invocations in a source string — the crude
/// "assertion density" signal (design.md §4: "crude counts are fine —
/// `assert*!` occurrences").
///
/// Counts an identifier immediately followed by `!` when the identifier is
/// `assert` / `debug_assert` or starts with `assert_` / `debug_assert_` (so
/// `assert!`, `assert_eq!`, `assert_ne!`, `assert_matches!`, `debug_assert!`,
/// … all count). Deliberately syntactic, not semantic: it does not parse Rust,
/// so an `assert!` inside a string literal or comment is counted too. That is
/// acceptable for a *relative* regression signal — the same crude rule applies
/// to the baseline and the current file, so systematic over-counting cancels;
/// only a real drop in assertions between the two moves the number.
#[must_use]
pub fn count_assert_macros(src: &str) -> usize {
    let bytes = src.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        // Find the start of an identifier (not preceded by an identifier char).
        if is_ident_start(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &src[start..i];
            if i < bytes.len() && bytes[i] == b'!' && is_assert_macro(ident) {
                count += 1;
            }
        } else {
            i += 1;
        }
    }
    count
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

    #[test]
    fn parses_mixed_libtest_outcomes() {
        let out = "\
running 5 tests
test export::csv::roundtrip ... ok
test export::csv::escaping ... FAILED
test routes::account::export_ok ... ok
test slow::network ... ignored
test slow::flaky ... ignored, needs network

failures:
    export::csv::escaping

test result: FAILED. 3 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out
";
        let snap = parse_libtest_output(out);
        assert_eq!(
            snap.passed,
            ["export::csv::roundtrip", "routes::account::export_ok"]
                .iter()
                .map(ToString::to_string)
                .collect()
        );
        assert_eq!(
            snap.failed,
            ["export::csv::escaping"]
                .iter()
                .map(ToString::to_string)
                .collect()
        );
        assert_eq!(snap.ignored.len(), 2);
        // The `test result:` tally is not mistaken for a test named `result:`.
        assert!(!snap.all_ids().iter().any(|id| id.contains("result")));
    }

    #[test]
    fn doc_test_lines_parse() {
        let out = "test src/lib.rs - foo (line 3) ... ok";
        let snap = parse_libtest_output(out);
        assert!(snap.passed.contains("src/lib.rs - foo (line 3)"));
    }

    #[test]
    fn empty_and_garbage_output_yield_empty_snapshot() {
        assert_eq!(parse_libtest_output("").total(), 0);
        assert_eq!(
            parse_libtest_output("Compiling foo v0.1.0\nnot a test line\n").total(),
            0
        );
    }

    #[test]
    fn ok_with_trailing_note_is_passed() {
        // `... ok (0.01s)`-style suffixes still classify as passing.
        let snap = parse_libtest_output("test a::b ... ok (fast)\n");
        assert!(snap.passed.contains("a::b"));
    }

    #[test]
    fn parses_clippy_short_warnings_excluding_tally() {
        let out = "\
    Checking foo v0.1.0
src/a.rs:3:9: warning: unused variable: `x`
src/b.rs:10:1: warning: function is never used: `helper`
warning: unused import: `std::io`
warning: `foo` (lib) generated 3 warnings
    Finished dev
";
        let snap = parse_clippy_short(out);
        assert_eq!(snap.warnings.len(), 3);
        assert!(snap
            .warnings
            .contains("src/a.rs:3:9: warning: unused variable: `x`"));
        assert!(snap.warnings.contains("warning: unused import: `std::io`"));
        // The `generated N warnings` tally is excluded.
        assert!(!snap.warnings.iter().any(|w| w.contains("generated")));
    }

    #[test]
    fn clippy_errors_are_not_warnings() {
        let out = "src/a.rs:1:1: error[E0433]: failed to resolve\n";
        assert!(parse_clippy_short(out).warnings.is_empty());
    }

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
        // `assertion_helper` is a call, not `assert*!`; `assert` without `!`
        // is a variable; `reassert!` does not start with the prefix.
        let src = "assertion_helper(); let assert = 1; reassert!(z); asserts!(w);";
        assert_eq!(count_assert_macros(src), 0);
    }

    #[test]
    fn counts_at_string_boundaries() {
        // Leading identifier and no trailing space before `!`.
        assert_eq!(count_assert_macros("assert!(true)"), 1);
        assert_eq!(count_assert_macros("x.assert_eq!(a,b)"), 1);
    }
}
