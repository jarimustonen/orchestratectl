//! `doctor` subcommand — read-only self-diagnostic (AGENTS-AI-FIRST-CLI
//! §18).
//!
//! Runs the full internal self-check and reports each finding so an agent
//! can answer "is the tool itself healthy?" in one call. Read-only by
//! default; the corrective twin `--fix` applies the safe subset of
//! suggestions, and `--fix --dry-run` shows that plan via the §11
//! planning envelope without touching anything.
//!
//! Output (`--output`, global):
//! - `text` — one line per check + `summary: N ok, M warn, K fail`.
//! - `json` — the §18 bundled shape: `{schema_version, data:{checks,
//!   summary}}` (plus `fixes_applied` under `--fix`).
//! - `jsonl` (default) — one check object per line, then a final
//!   `{"event":"summary", ...}` line. This streaming shape deviates from
//!   §18's bundled-array example on purpose: jsonl semantics across this
//!   binary are one-event-per-line. The bundled array is still available
//!   via `--output json`.
//!
//! Exit code: 0 when all checks are `ok`/`warn`; 1 when any check is
//! `fail`. `--fix --dry-run` always exits 0 (the plan itself succeeded).

mod check;
mod checks;
mod fix;

use std::io::Write;
use std::process::ExitCode;

use serde::Serialize;
use serde_json::json;

use crate::error::CliError;
use crate::output::{OutputFormat, OutputSpec};

use check::{CheckResult, Summary};
use checks::Ctx;

#[derive(clap::Args, Debug)]
pub struct DoctorArgs {
    /// Apply the safe subset of `fix_suggestion`s after running the
    /// checks. Opt-in per invocation; never the default (§18).
    #[arg(long)]
    pub fix: bool,
    /// With `--fix`, print the planned fixes via the §11 planning
    /// envelope and apply nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Entry point. Returns an [`ExitCode`] directly (rather than the usual
/// `Result<(), CliError>`) because §18 exit semantics — 1 on any `fail`
/// without an error envelope — do not map onto the shared error path.
pub fn run(args: &DoctorArgs, spec: &OutputSpec, warnings: &[String]) -> ExitCode {
    match run_inner(args, spec, warnings) {
        Ok(code) => code,
        Err(e) => {
            e.emit();
            ExitCode::from(e.kind as u8)
        }
    }
}

fn run_inner(
    args: &DoctorArgs,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<ExitCode, CliError> {
    if args.dry_run && !args.fix {
        return Err(CliError::user(
            "invalid_arguments",
            "--dry-run only applies with --fix (doctor is read-only by default)",
        ));
    }

    // Consume the already-resolved Taskfleet home. `None` is retained as a
    // defensive diagnostic seam for direct test/library calls.
    let root = crate::home::root_dir().ok();
    let ctx = Ctx { root };

    let results = checks::run_all(&ctx);
    let summary = Summary::tally(&results);

    if args.fix && args.dry_run {
        emit_dry_run(&results, spec, warnings)?;
        // The plan itself succeeded; §11 dry-run exits 0 regardless of
        // the underlying check statuses.
        return Ok(ExitCode::SUCCESS);
    }

    let applied = if args.fix {
        Some(fix::apply(&results))
    } else {
        None
    };

    emit_report(&results, &summary, applied.as_deref(), spec, warnings)?;

    Ok(if summary.any_fail() {
        // §18: any fail → exit 1, with no error envelope (the report on
        // stdout *is* the answer).
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Render the §11 planning envelope for `--fix --dry-run`.
fn emit_dry_run(
    results: &[CheckResult],
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let plan = fix::plan(results);
    match spec.format {
        OutputFormat::Text => {
            render_text_checks(results);
            // Mirror the real-run summary so a human (or text-parsing agent)
            // sees the fail/warn tally even in dry-run, where the exit code
            // is always 0.
            let summary = Summary::tally(results);
            println!(
                "summary: {} ok, {} warn, {} fail",
                summary.ok, summary.warn, summary.fail
            );
            if plan.is_empty() {
                println!("dry-run: no safe fixes to apply");
            } else {
                println!("dry-run: would apply {} fix(es):", plan.len());
                for p in &plan {
                    println!(
                        "  would {} {} {} (for {})",
                        p.action, p.resource, p.target, p.check_id
                    );
                }
            }
            emit_text_warnings(warnings);
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            // §11 planning envelope — distinct from the success envelope.
            let mut envelope = json!({
                "schema_version": taskfleet_core::SCHEMA_VERSION,
                "dry_run": true,
                "would": plan,
            });
            if !warnings.is_empty() {
                envelope["warnings"] = json!(warnings);
            }
            let pretty = matches!(spec.format, OutputFormat::Json);
            write_json_doc(&envelope, spec, pretty)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ReportPayload<'a> {
    checks: &'a [CheckResult],
    summary: Summary,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixes_applied: Option<&'a [fix::AppliedFix]>,
}

/// Render the check report (and, under `--fix`, the applied fixes) in the
/// requested format.
fn emit_report(
    results: &[CheckResult],
    summary: &Summary,
    applied: Option<&[fix::AppliedFix]>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Text => {
            render_text_checks(results);
            if let Some(fixes) = applied {
                render_text_fixes(fixes);
            }
            println!(
                "summary: {} ok, {} warn, {} fail",
                summary.ok, summary.warn, summary.fail
            );
            emit_text_warnings(warnings);
        }
        OutputFormat::Json => {
            let payload = ReportPayload {
                checks: results,
                summary: *summary,
                fixes_applied: applied,
            };
            // §18 bundled shape under the standard `data` envelope.
            crate::output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Jsonl => {
            render_jsonl(results, summary, applied, warnings, spec)?;
        }
    }
    Ok(())
}

fn render_text_checks(results: &[CheckResult]) {
    for r in results {
        let tag = match r.status {
            check::CheckStatus::Ok => "OK  ",
            check::CheckStatus::Warn => "WARN",
            check::CheckStatus::Fail => "FAIL",
        };
        match &r.fix_suggestion {
            Some(fix) => println!("{tag} {}  {}  (fix: {fix})", r.id, r.message),
            None => println!("{tag} {}  {}", r.id, r.message),
        }
    }
}

fn render_text_fixes(fixes: &[fix::AppliedFix]) {
    for f in fixes {
        let tag = if f.applied { "FIXED " } else { "FAILED" };
        println!(
            "{tag} {} {} {}  {}",
            f.check_id, f.action, f.target, f.message
        );
    }
}

/// Streaming jsonl: one self-describing object per line. Every line
/// carries `schema_version` and an `event` discriminator
/// (`check`/`fix`/`summary`) so a streaming consumer can identify and
/// version-check each record independently — the bundled `{schema_version,
/// data}` envelope is only available via `--output json`.
fn render_jsonl(
    results: &[CheckResult],
    summary: &Summary,
    applied: Option<&[fix::AppliedFix]>,
    warnings: &[String],
    spec: &OutputSpec,
) -> Result<(), CliError> {
    let mut buf = String::new();
    let mut push = |mut v: serde_json::Value, event: &str| -> Result<(), CliError> {
        let obj = v.as_object_mut().ok_or_else(|| {
            CliError::system("internal_serialize", "jsonl event was not an object")
        })?;
        obj.insert("event".into(), json!(event));
        obj.insert(
            "schema_version".into(),
            json!(taskfleet_core::SCHEMA_VERSION),
        );
        buf.push_str(&serde_json::Value::Object(obj.clone()).to_string());
        buf.push('\n');
        Ok(())
    };
    for r in results {
        let v = serde_json::to_value(r)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
        push(v, "check")?;
    }
    if let Some(fixes) = applied {
        for f in fixes {
            let v = serde_json::to_value(f)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            push(v, "fix")?;
        }
    }
    let mut summary_event = json!({
        "ok": summary.ok,
        "warn": summary.warn,
        "fail": summary.fail,
    });
    if !warnings.is_empty() {
        summary_event["warnings"] = json!(warnings);
    }
    push(summary_event, "summary")?;
    write_str(&buf, spec)
}

/// Write a single JSON document (compact or pretty) to the resolved
/// destination, with a trailing newline.
fn write_json_doc(
    value: &serde_json::Value,
    spec: &OutputSpec,
    pretty: bool,
) -> Result<(), CliError> {
    let mut s = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    s.push('\n');
    write_str(&s, spec)
}

/// Honour `--output PATH` redirection (the machine envelope goes to the
/// file) the same way `output::emit_envelope` does; otherwise stdout.
fn write_str(s: &str, spec: &OutputSpec) -> Result<(), CliError> {
    match &spec.file {
        None => {
            let mut out = std::io::stdout().lock();
            out.write_all(s.as_bytes())
                .map_err(|e| CliError::system("io_error", format!("write stdout: {e}")))?;
            out.flush()
                .map_err(|e| CliError::system("io_error", format!("flush stdout: {e}")))?;
        }
        Some(path) => {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .map_err(|e| {
                    CliError::system("io_error", format!("open {}: {e}", path.display()))
                })?;
            f.write_all(s.as_bytes()).map_err(|e| {
                CliError::system("io_error", format!("write {}: {e}", path.display()))
            })?;
            f.flush().map_err(|e| {
                CliError::system("io_error", format!("flush {}: {e}", path.display()))
            })?;
        }
    }
    Ok(())
}

fn emit_text_warnings(warnings: &[String]) {
    crate::output::emit_text_warnings(warnings);
}
