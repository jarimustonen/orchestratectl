//! Stdout success-payload helpers and the global `--output` format model.
//!
//! Every machine-readable payload is shaped as:
//!
//! ```json
//! {"schema_version": 1, "data": {...subcommand body...}, "warnings": [...]?}
//! ```
//!
//! The body lives under a dedicated `data` key so the envelope can grow
//! reserved fields (`warnings`, `dry_run`, `trace_id`, ...) over time
//! without colliding with payload field names.
//!
//! The `--output` flag (per AGENTS-AI-FIRST-CLI §9 + §13) is the single
//! switch that selects between the three rendering modes — `text`,
//! `json`, `jsonl` — and optionally redirects the machine envelope to a
//! file (`--output PATH.jsonl` / `--output PATH.json`).

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
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    warnings: &'a [String],
}

/// Build the warning string for `dropped` lossy-mode log-event drops, or
/// `None` when nothing was dropped (the steady state). Kept separate from
/// [`emit_envelope`] so the count→message mapping is unit-testable without a
/// live appender. Plain-string form matches every other entry in the
/// `warnings` array (the array is `[String]`, not structured objects); the
/// count is embedded so an agent can still read it.
fn dropped_log_warning(dropped: u64) -> Option<String> {
    (dropped > 0).then(|| {
        format!(
            "{dropped} log event(s) dropped due to buffer overflow \
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
    // Append the dropped-event warning (if any) to the caller's base
    // warnings. Allocate only when there is something to add — the common
    // case (no drops) borrows the caller's slice unchanged.
    let augmented: Vec<String>;
    let warnings: &[String] = match dropped_log_warning(dropped) {
        Some(w) => {
            augmented = warnings.iter().cloned().chain(std::iter::once(w)).collect();
            &augmented
        }
        None => warnings,
    };
    let envelope = SuccessEnvelope {
        schema_version: SCHEMA_VERSION,
        data: body,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
