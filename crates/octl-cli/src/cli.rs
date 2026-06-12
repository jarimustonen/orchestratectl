//! Clap-based CLI dispatch.
//!
//! MVP only ships a placeholder `version` subcommand. The full subcommand
//! tree (per `design.md` §2) lands in subsequent issues.

use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{ColorChoice, Parser, Subcommand};
use serde::Serialize;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::error::{CliError, ExitKind};
use crate::output;

const GIT_COMMIT: &str = env!("ORCHESTRATECTL_GIT_COMMIT");
const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "orchestratectl",
    version = CARGO_VERSION,
    about = "Orchestrate AI-agent workflows: worktrees, fan-out, orchestrate, llm-skills.",
    disable_help_subcommand = true,
    disable_version_flag = true,
    color = ColorChoice::Never,
)]
struct Cli {
    /// Emit machine-readable JSON output on stdout. Shorthand for the
    /// `--output` flag added in a follow-up issue.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show binary, commit, and state-schema versions.
    Version,
}

pub fn run() -> ExitCode {
    let logging_warnings = init_logging();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return handle_clap_error(e, &logging_warnings),
    };

    info!(
        target: "orchestratectl::cli",
        json = cli.json,
        command = ?cli.command,
        "command dispatched"
    );

    let result = match cli.command {
        Command::Version => cmd_version(cli.json, &logging_warnings),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            e.emit();
            ExitCode::from(e.kind as u8)
        }
    }
}

fn handle_clap_error(e: clap::Error, logging_warnings: &[String]) -> ExitCode {
    use clap::error::ErrorKind;
    // Help is not a failure; let clap print and exit 0. `--version` is
    // disabled at the clap level (`disable_version_flag`), so it never
    // surfaces here — agents must use the `version` subcommand.
    if matches!(
        e.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
        let _ = e.print();
        return ExitCode::SUCCESS;
    }

    // Preserve the full clap error context (allowed values, usage hints).
    // §4 of AGENTS-AI-FIRST-CLI requires the expected format to reach the
    // caller; taking only `.lines().next()` strips that. We trim the
    // trailing "For more information, try '--help'." line because it
    // depends on TTY and is noise for the JSON envelope.
    let message = e
        .to_string()
        .lines()
        .filter(|l| !l.trim_start().starts_with("For more information"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let message = if message.is_empty() {
        "invalid arguments".to_string()
    } else {
        message
    };

    let code = match e.kind() {
        ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument => "unknown_subcommand_or_flag",
        ErrorKind::MissingRequiredArgument | ErrorKind::MissingSubcommand => "missing_argument",
        ErrorKind::InvalidValue => "invalid_value",
        _ => "invalid_arguments",
    };
    let err = CliError {
        kind: ExitKind::User,
        code: code.to_string(),
        message,
        invalid_value: None,
        expected: None,
    };
    err.emit();
    for w in logging_warnings {
        eprintln!("warning: {}", w);
    }
    ExitCode::from(ExitKind::User as u8)
}

#[derive(Debug, Serialize)]
struct VersionPayload {
    version: &'static str,
    commit: &'static str,
    schema_version: u32,
    supported_schemas: &'static [u32],
    state_schema_version: u32,
    supported_state_schemas: &'static [u32],
}

fn cmd_version(json: bool, warnings: &[String]) -> Result<(), CliError> {
    let payload = VersionPayload {
        version: CARGO_VERSION,
        commit: GIT_COMMIT,
        schema_version: crate::error::SCHEMA_VERSION,
        supported_schemas: &[crate::error::SCHEMA_VERSION],
        state_schema_version: octl_core::STATE_SCHEMA_VERSION,
        supported_state_schemas: octl_core::SUPPORTED_STATE_SCHEMAS,
    };
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("orchestratectl {}", payload.version);
        println!("commit:               {}", payload.commit);
        println!("envelope schema:      {}", payload.schema_version);
        println!("state schema version: {}", payload.state_schema_version);
        println!(
            "supported state:      {:?}",
            payload.supported_state_schemas
        );
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

/// Initialise the JSONL log subscriber. Logs go to
/// `~/.orchestratectl/logs/orchestratectl.log.jsonl`. Best-effort: if the
/// log file cannot be opened, the process still runs and the caller sees
/// the failure in the success-envelope `warnings` array or on stderr.
fn init_logging() -> Vec<String> {
    let mut warnings = Vec::new();
    let log_path = match log_path() {
        Some(p) => p,
        None => {
            warnings
                .push("log path unavailable: HOME and ORCHESTRATECTL_HOME both unset".to_string());
            return warnings;
        }
    };

    if let Some(parent) = log_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            warnings.push(format!(
                "could not create log directory {}: {}",
                parent.display(),
                e
            ));
            return warnings;
        }
    }

    let file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!(
                "could not open log file {}: {}",
                log_path.display(),
                e
            ));
            return warnings;
        }
    };

    let filter =
        EnvFilter::try_from_env("ORCHESTRATECTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    // Share one open file handle across every log event via Arc<File>.
    // tracing-subscriber's `MakeWriter` impl for `Arc<File>` writes
    // through `&File` (one `write(2)` per event under the kernel's
    // O_APPEND atomicity guarantee) — no per-event `try_clone` and no
    // panicking path on FD exhaustion.
    let writer: Arc<File> = Arc::new(file);
    let layer = fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(writer);

    if tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .is_err()
    {
        warnings.push("tracing subscriber already initialised".to_string());
    }

    warnings
}

fn log_path() -> Option<PathBuf> {
    let root = if let Ok(custom) = std::env::var("ORCHESTRATECTL_HOME") {
        PathBuf::from(custom)
    } else {
        let home = std::env::var("HOME").ok()?;
        PathBuf::from(home).join(".orchestratectl")
    };
    Some(root.join("logs").join("orchestratectl.log.jsonl"))
}
