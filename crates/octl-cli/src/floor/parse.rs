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
        // Skip the `test result:` tally before anything else — it starts with
        // `test ` but is not a per-test line.
        if line.starts_with("test result:") {
            continue;
        }
        // Must be `test <name> ... <outcome>`.
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
/// "... generated N warnings" summary) is excluded so the count itself is not
/// mistaken for a warning.
///
/// **Warning identity strips the `:line:col` span** ([`normalize_clippy_line`]):
/// the identity is `path: warning: message`, not `path:line:col: warning:
/// message`. Without this, inserting a line above an *unchanged* warning shifts
/// its line number, so a byte-exact set-difference would report the shifted
/// warning as "new" and the original as "gone" — failing the merge on a
/// behaviour-preserving edit. The trade-off: two occurrences of the same lint
/// with the same message in the same file collapse to one identity, so a *new*
/// same-message occurrence is not flagged. That narrow miss is preferable to
/// blocking every line-shifting refactor; the proper fix (structured
/// `--message-format=json` with the lint code as identity) is tracked in
/// `floor-capture-trust-model`.
///
/// Errors (`error:` / `error[E….]:`) are not collected — the floor's clippy
/// gate is specifically "no new *warnings*"; a hard error fails the build (and
/// the relevant check) on its own.
///
/// NOTE: this parses uncontrolled process text, so a line the code under review
/// prints (e.g. via `println!` or a `build.rs`) that looks like a diagnostic is
/// indistinguishable from a real one. That injection surface is the subject of
/// `floor-capture-trust-model`; text parsing is an interim, advisory capture.
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
        snap.warnings.insert(normalize_clippy_line(line));
    }
    snap
}

/// Strip a leading `path:line:col:` span down to `path:` so a warning's identity
/// does not shift when unrelated edits move its line number. A line without the
/// numeric `line:col` prefix (a crate-level `warning: …`) is returned unchanged.
#[must_use]
pub fn normalize_clippy_line(line: &str) -> String {
    // Short format is `path:line:col: warning: message`. Split into at most 4
    // pieces on `:` and, only when the 2nd and 3rd are integers (the span),
    // drop them; otherwise leave the line as-is (paths can't be a bare integer,
    // so this never mangles a crate-level `warning:` line).
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() == 4
        && parts[1].trim().parse::<u32>().is_ok()
        && parts[2].trim().parse::<u32>().is_ok()
    {
        format!("{}:{}", parts[0], parts[3])
    } else {
        line.to_string()
    }
}

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
/// `assert!(true)` from a real assertion, and the stripper is a lexer, not a
/// full parser (e.g. `assert! /*c*/ (x)` with whitespace/comments before `!` is
/// missed). It is a *relative* regression signal, not an injection-proof oracle
/// — an adversary that writes the code can still hold the number while gutting
/// coverage. Hardening this to a semantic, per-test measure is tracked in
/// `floor-capture-trust-model`.
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
        // Identity has the `:line:col` span stripped.
        assert!(snap
            .warnings
            .contains("src/a.rs: warning: unused variable: `x`"));
        assert!(snap.warnings.contains("warning: unused import: `std::io`"));
        // The `generated N warnings` tally is excluded.
        assert!(!snap.warnings.iter().any(|w| w.contains("generated")));
    }

    #[test]
    fn clippy_identity_is_stable_across_line_shifts() {
        // The same warning at a different line yields the same identity, so a
        // line-shifting edit above it is not reported as a "new" warning.
        let before = parse_clippy_short("src/a.rs:3:9: warning: unused variable: `x`\n");
        let after = parse_clippy_short("src/a.rs:47:9: warning: unused variable: `x`\n");
        assert_eq!(before.warnings, after.warnings);
    }

    #[test]
    fn normalize_clippy_line_leaves_crate_level_and_odd_lines_intact() {
        assert_eq!(
            normalize_clippy_line("warning: unused import: `std::io`"),
            "warning: unused import: `std::io`"
        );
        // Not a `path:line:col:` prefix (no integers) → unchanged.
        assert_eq!(normalize_clippy_line("weird: line"), "weird: line");
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

    #[test]
    fn does_not_count_assertions_in_comments_or_strings() {
        // The anti-gaming case: padding the count with commented/quoted
        // `assert!` no longer works.
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
        // Only the single real `assert!(real)` counts.
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
        // `'a` (lifetime) has no closing quote; stripping must not swallow the
        // rest of the line, so the assertion after it still counts.
        let src = "fn f<'a>(x: &'a T) { assert!(x); }";
        assert_eq!(count_assert_macros(src), 1);
    }
}
