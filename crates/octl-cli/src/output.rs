//! Stdout success-payload helpers and the global `--output` format model.
//!
//! Every machine-readable payload is shaped as:
//!
//! ```json
//! {"schema_version": 1, "data": {...subcommand body...},
//!  "dropped_log_events": 7?, "warnings": [...]?}
//! ```
//!
//! The body lives under a dedicated `data` key so the envelope can grow
//! reserved fields (`warnings`, `dropped_log_events`, `dry_run`,
//! `trace_id`, ...) over time without colliding with payload field names.
//!
//! The `--output` flag (per AGENTS-AI-FIRST-CLI §9 + §13) is the single
//! switch that selects between the three rendering modes — `text`,
//! `json`, `jsonl` — and optionally redirects the machine envelope to a
//! file (`--output PATH.jsonl` / `--output PATH.json`).

use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use octl_core::SCHEMA_VERSION;

use crate::error::CliError;

/// Resolved output format for a single invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable rendering (per-command).
    Text,
    /// Pretty-printed single JSON document.
    Json,
    /// Compact one-JSON-object-per-line stream (default — AI-first).
    #[default]
    Jsonl,
}

/// Fully resolved `--output` spec: a format plus an optional file
/// destination. When `file` is `Some`, the machine envelope is written to
/// the file; text rendering is incompatible with a file destination (the
/// path's extension determines the format and is always `json`/`jsonl`).
#[derive(Debug, Clone)]
pub struct OutputSpec {
    pub format: OutputFormat,
    pub file: Option<PathBuf>,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Jsonl,
            file: None,
        }
    }
}

/// Custom clap value parser for the global `--output` flag.
///
/// Accepts:
/// - exact tokens `text`, `json`, `jsonl` (format selector)
/// - a path-shaped value (starts with `/`, `.`, or contains `/`) whose
///   extension is `.json` or `.jsonl` (file destination + inferred format)
///
/// Anything else is rejected with `invalid_value`.
pub fn parse_output_value(s: &str) -> Result<OutputSpec, String> {
    match s {
        "text" => Ok(OutputSpec {
            format: OutputFormat::Text,
            file: None,
        }),
        "json" => Ok(OutputSpec {
            format: OutputFormat::Json,
            file: None,
        }),
        "jsonl" => Ok(OutputSpec {
            format: OutputFormat::Jsonl,
            file: None,
        }),
        _ if looks_like_path(s) => {
            let path = PathBuf::from(s);
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            let format = match ext.as_deref() {
                Some("jsonl") => OutputFormat::Jsonl,
                Some("json") => OutputFormat::Json,
                _ => {
                    return Err(format!("file path '{s}' must end in .json or .jsonl"));
                }
            };
            Ok(OutputSpec {
                format,
                file: Some(path),
            })
        }
        _ => Err(format!(
            "expected one of text|json|jsonl or a .json/.jsonl file path; got '{s}'"
        )),
    }
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('.') || s.contains('/')
}

#[derive(Serialize)]
struct SuccessEnvelope<'a, T: Serialize> {
    schema_version: u32,
    data: &'a T,
    /// Process-cumulative count of log events dropped by the lossy
    /// non-blocking appender (buffer overflow). The machine-readable
    /// companion to the human-readable `warnings` entry, so agents read a
    /// number instead of regex-parsing prose. Omitted (and the warning
    /// suppressed) when zero, so the field is purely additive — no
    /// `schema_version` bump (AGENTS-AI-FIRST-CLI §10: new fields are
    /// additive). See [`dropped_log_warning`].
    #[serde(skip_serializing_if = "Option::is_none")]
    dropped_log_events: Option<u64>,
    /// Always emitted (even when empty), per AGENTS-AI-FIRST-CLI §10: a
    /// missing-vs-empty branch is a consumer tax — agents would have to
    /// read `warnings` as `Vec<String> | undefined`. `warnings: []` is
    /// the steady state.
    warnings: &'a [String],
}

/// Build the warning string for `dropped` lossy-mode log-event drops, or
/// `None` when nothing was dropped (the steady state). Kept separate from
/// [`emit_envelope`] so the count→message mapping is unit-testable without a
/// live appender.
///
/// This is the *human* rendering; agents should prefer the structured
/// `dropped_log_events` envelope field (and the supervisor's structured
/// `warn!` fields). The count is process-cumulative since logging init.
fn dropped_log_warning(dropped: u64) -> Option<String> {
    (dropped > 0).then(|| {
        let events = if dropped == 1 { "event" } else { "events" };
        format!(
            "{dropped} log {events} dropped due to buffer overflow \
             (lossy non-blocking appender under sustained back-pressure)"
        )
    })
}

/// Serialize `body` inside the canonical success envelope and emit it
/// according to `spec`. `OutputFormat::Text` is a programmer error — the
/// caller is responsible for text rendering (this helper covers only the
/// JSON envelope branches).
///
/// This is the single chokepoint where the process's lossy-mode dropped-event
/// count ([`crate::cli::dropped_log_events`]) is folded into the envelope's
/// `warnings`: a subcommand renders its envelope *after* doing its work, so
/// this is the first place the final count is known. Long-lived commands that
/// never render an envelope (`event tail --follow`, `supervise`) surface drops
/// via a periodic `warn!` instead.
pub fn emit_envelope<T: Serialize>(
    body: &T,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    emit_envelope_with_dropped(body, spec, warnings, crate::cli::dropped_log_events())
}

/// [`emit_envelope`] with the dropped-event count injected explicitly, so the
/// drop→warning→serialization path is testable without a live appender.
fn emit_envelope_with_dropped<T: Serialize>(
    body: &T,
    spec: &OutputSpec,
    warnings: &[String],
    dropped: u64,
) -> Result<(), CliError> {
    // Surface drops two ways: the structured `dropped_log_events` field (for
    // agents) and a human-readable `warnings` entry. Allocate the warnings
    // vec only when there is something to add — the common case (no drops)
    // borrows the caller's slice unchanged.
    let augmented: Vec<String>;
    let (warnings, dropped_log_events): (&[String], Option<u64>) =
        match dropped_log_warning(dropped) {
            Some(w) => {
                augmented = warnings.iter().cloned().chain(std::iter::once(w)).collect();
                (&augmented, Some(dropped))
            }
            None => (warnings, None),
        };
    let envelope = SuccessEnvelope {
        schema_version: SCHEMA_VERSION,
        data: body,
        dropped_log_events,
        warnings,
    };
    let bytes = match spec.format {
        OutputFormat::Jsonl => {
            let mut s = serde_json::to_string(&envelope)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            s.push('\n');
            s.into_bytes()
        }
        OutputFormat::Json => {
            let mut s = serde_json::to_string_pretty(&envelope)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            s.push('\n');
            s.into_bytes()
        }
        OutputFormat::Text => {
            return Err(CliError::system(
                "internal_format_mismatch",
                "emit_envelope called in text mode",
            ));
        }
    };
    match &spec.file {
        None => {
            let mut out = std::io::stdout().lock();
            out.write_all(&bytes)
                .map_err(|e| CliError::system("io_error", format!("write stdout: {e}")))?;
            out.flush()
                .map_err(|e| CliError::system("io_error", format!("flush stdout: {e}")))?;
        }
        Some(path) => {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .map_err(|e| {
                    CliError::system("io_error", format!("open {}: {}", path.display(), e))
                })?;
            f.write_all(&bytes).map_err(|e| {
                CliError::system("io_error", format!("write {}: {}", path.display(), e))
            })?;
            f.flush().map_err(|e| {
                CliError::system("io_error", format!("flush {}: {}", path.display(), e))
            })?;
        }
    }
    Ok(())
}

/// Emit trailing text-mode warnings (each on its own `warning: ` line on
/// stderr). Shared across every subcommand's text branch. The lossy-mode
/// dropped-event warning is appended here too, so text and JSON renderings
/// surface the same drops (mirrors [`emit_envelope`]).
pub fn emit_text_warnings(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
    if let Some(w) = dropped_log_warning(crate::cli::dropped_log_events()) {
        eprintln!("warning: {w}");
    }
}

/// Escape ASCII control characters in a free-form, user-controllable string
/// so it can be printed on a single physical line of `--format text` output
/// without spoofing rows/columns or breaking field alignment.
///
/// `\n`, `\t`, `\r` render as their familiar two-char escapes; every other
/// ASCII control char (NUL, vertical-tab, form-feed, the rest of C0, and DEL
/// `0x7F`) renders as `\xNN`. Non-control characters — including all
/// multi-byte UTF-8 — pass through untouched.
///
/// Returns `Cow::Borrowed` (no allocation) on the steady-state path where the
/// input is already clean. Apply this only to user-controllable text fields
/// (`topic`, `context`, `note`, `choice`, `severity`, `title`, …) — never to
/// operator-facing identifiers the user is meant to see verbatim (run-ids,
/// file paths, branch/tmux names already escaped upstream).
pub fn escape_one_line(s: &str) -> Cow<'_, str> {
    // Fast path: an ASCII control byte is always `< 0x20` or `== 0x7F`. UTF-8
    // continuation/lead bytes are all `>= 0x80`, so a raw byte scan never
    // misclassifies a multi-byte char — and lets the clean case return without
    // touching the heap.
    if !s.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                // C0 controls and DEL are all single-byte (`<= 0x7F`), so the
                // value fits in two uppercase hex digits. Push them directly
                // rather than via `format!` to avoid an interim allocation.
                let b = c as u8;
                out.push('\\');
                out.push('x');
                out.push(hex_upper(b >> 4));
                out.push(hex_upper(b & 0x0f));
            }
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Map a 4-bit nibble (`0..=15`) to its uppercase ASCII hex digit.
fn hex_upper(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16)
        .expect("nibble is < 16")
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_one_line_maps_each_control_char() {
        assert_eq!(escape_one_line("a\nb"), "a\\nb");
        assert_eq!(escape_one_line("a\tb"), "a\\tb");
        assert_eq!(escape_one_line("a\rb"), "a\\rb");
        // Vertical tab, form-feed, NUL, and a generic C0 control render as \xNN.
        assert_eq!(escape_one_line("a\u{0b}b"), "a\\x0Bb");
        assert_eq!(escape_one_line("a\u{0c}b"), "a\\x0Cb");
        assert_eq!(escape_one_line("a\0b"), "a\\x00b");
        assert_eq!(escape_one_line("a\u{01}b"), "a\\x01b");
        // DEL (0x7F) is a control char too.
        assert_eq!(escape_one_line("a\u{7f}b"), "a\\x7Fb");
    }

    #[test]
    fn escape_one_line_clean_input_borrows() {
        // Steady-state path: clean input (including multi-byte UTF-8) must not
        // allocate — it borrows the original.
        for clean in ["", "plain text", "emoji 🎬 and åäö"] {
            assert!(
                matches!(escape_one_line(clean), Cow::Borrowed(_)),
                "clean input must borrow: {clean:?}"
            );
        }
        // A string that needs escaping must own.
        assert!(matches!(escape_one_line("x\ny"), Cow::Owned(_)));
    }

    #[test]
    fn parses_format_tokens() {
        assert_eq!(
            parse_output_value("text").unwrap().format,
            OutputFormat::Text
        );
        assert_eq!(
            parse_output_value("json").unwrap().format,
            OutputFormat::Json
        );
        assert_eq!(
            parse_output_value("jsonl").unwrap().format,
            OutputFormat::Jsonl
        );
    }

    #[test]
    fn parses_file_paths() {
        let s = parse_output_value("./out.jsonl").unwrap();
        assert_eq!(s.format, OutputFormat::Jsonl);
        assert_eq!(s.file.as_deref().unwrap().to_str().unwrap(), "./out.jsonl");

        let s = parse_output_value("/tmp/x.json").unwrap();
        assert_eq!(s.format, OutputFormat::Json);

        let s = parse_output_value("dir/sub/file.jsonl").unwrap();
        assert_eq!(s.format, OutputFormat::Jsonl);
    }

    #[test]
    fn rejects_unknown_token() {
        assert!(parse_output_value("yaml").is_err());
        assert!(parse_output_value("").is_err());
        // `text` and `json` only as exact tokens — file path needs extension
        assert!(parse_output_value("./out.txt").is_err());
        assert!(parse_output_value("./noext").is_err());
    }

    #[test]
    fn dropped_log_warning_present_only_when_nonzero() {
        assert!(dropped_log_warning(0).is_none(), "no warning at zero drops");
        let w = dropped_log_warning(42).expect("warning at nonzero drops");
        assert!(
            w.contains("42"),
            "count must be embedded in the message: {w}"
        );
        assert!(
            w.contains("buffer overflow"),
            "message must explain the cause: {w}"
        );
        // Grammar: singular for exactly one, plural otherwise.
        assert!(
            dropped_log_warning(1).unwrap().contains("1 log event "),
            "singular grammar for one drop"
        );
        assert!(
            dropped_log_warning(2).unwrap().contains("2 log events "),
            "plural grammar for many"
        );
    }

    /// The dropped-event count must be visible in the rendered success
    /// envelope — both as the structured `dropped_log_events` field (for
    /// agents) and as a `warnings` entry (for humans). Drives the real
    /// serialization path via [`emit_envelope_with_dropped`] to a temp file,
    /// then parses it back. Base warnings are preserved before the drop one.
    #[test]
    fn dropped_count_visible_in_success_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let spec = OutputSpec {
            format: OutputFormat::Json,
            file: Some(path.clone()),
        };
        let body = serde_json::json!({"ok": true});
        emit_envelope_with_dropped(&body, &spec, &["base warning".to_string()], 7).unwrap();

        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // Structured field — the machine-readable contract.
        assert_eq!(
            v["dropped_log_events"].as_u64(),
            Some(7),
            "structured dropped count not visible: {v}"
        );
        // Human-readable warnings, base preserved first.
        let warnings: Vec<&str> = v["warnings"]
            .as_array()
            .expect("warnings array present")
            .iter()
            .map(|w| w.as_str().expect("warning is a string"))
            .collect();
        assert_eq!(
            warnings.len(),
            2,
            "base warning + dropped-event warning: {warnings:?}"
        );
        assert_eq!(warnings[0], "base warning", "base warnings preserved first");
        assert!(
            warnings[1].contains("7 log events"),
            "dropped count not visible in envelope warnings: {warnings:?}"
        );
    }

    /// Zero drops must omit the structured `dropped_log_events` field, but
    /// `warnings` is always emitted as `[]` per AGENTS-AI-FIRST-CLI §10 so
    /// agents don't pay a missing-vs-empty branch tax.
    #[test]
    fn no_dropped_warning_when_count_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let spec = OutputSpec {
            format: OutputFormat::Json,
            file: Some(path.clone()),
        };
        let body = serde_json::json!({"ok": true});
        emit_envelope_with_dropped(&body, &spec, &[], 0).unwrap();

        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            v.get("warnings"),
            Some(&serde_json::json!([])),
            "warnings must always be rendered (empty array when no warnings): {v}"
        );
        assert!(
            v.get("dropped_log_events").is_none(),
            "zero drops must omit the structured field: {v}"
        );
    }
}
