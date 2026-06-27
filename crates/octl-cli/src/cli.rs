//! Clap-based CLI dispatch.
//!
//! MVP only ships a placeholder `version` subcommand. The full subcommand
//! tree (per `design.md` §2) lands in subsequent issues.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{ColorChoice, Parser, Subcommand};
use serde::Serialize;
use tracing::info;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::error::{CliError, ExitKind};
use crate::output::{self, OutputFormat, OutputSpec};

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
    /// Output format. `jsonl` (default) emits one compact JSON envelope
    /// per line on stdout — AI-first. `json` emits a single pretty
    /// document. `text` emits a human-readable summary. A path-shaped
    /// value (`./out.json`, `./out.jsonl`) routes the machine envelope
    /// to that file (the format is inferred from the extension).
    #[arg(
        long,
        global = true,
        default_value = "jsonl",
        value_name = "FMT|PATH",
        value_parser = parse_output_arg,
    )]
    output: OutputSpec,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show binary, commit, and state-schema versions.
    Version,
    /// List, show, or install companion AI-skills shipped with this binary.
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Create, list, show, cancel, or reattach a run.
    Run {
        #[command(subcommand)]
        action: crate::run::RunAction,
    },
    /// Read (`tail`) or append (`create`) events on a run's event log.
    Event {
        #[command(subcommand)]
        action: crate::event::EventAction,
    },
    /// List nodes, show a node, or submit a structured terminal report.
    Node {
        #[command(subcommand)]
        action: crate::node::NodeAction,
    },
    /// List, show, or resolve discussions for a run.
    Discussion {
        #[command(subcommand)]
        action: crate::discussion::DiscussionAction,
    },
    /// List, approve, or reject spin-off proposals on a run.
    Spinoff {
        #[command(subcommand)]
        action: crate::spinoff::SpinoffAction,
    },
    /// Long-lived per-run supervisor: tail-follow events, watchdog
    /// agents, consume child `node.report` events with deterministic-
    /// ID dedup. Re-enters the same binary; `run reattach` invokes it.
    Supervise(crate::supervise::SuperviseArgs),
    /// Read-only self-diagnostic: validate schema, skill-sync, deps,
    /// config, and data integrity. `--fix` applies the safe subset.
    Doctor(crate::doctor::DoctorArgs),
}

#[derive(Subcommand, Debug)]
enum SkillAction {
    /// List skills embedded in this binary.
    List,
    /// Print a skill's SKILL.md to stdout.
    Show {
        /// Skill name (see `skill list`).
        name: String,
    },
    /// Stream a skill's SKILL.md (frontmatter + body) byte-identically
    /// to stdout. Read-only twin of `install` (AGENTS-AI-FIRST-CLI §16).
    Print {
        /// Skill name (see `skill list`).
        name: String,
    },
    /// Copy a skill's SKILL.md to the agent's skill directory. Installs
    /// every embedded skill when no name is given (per §15).
    Install {
        /// Skill name (see `skill list`). Omit to install every skill.
        name: Option<String>,
        /// Which agent runtime to install for.
        #[arg(long, value_enum, default_value_t = SkillAgentArg::Claude)]
        agent: SkillAgentArg,
        /// Override the destination path. Incompatible with `--agent all`
        /// and with the install-all (no-name) form.
        #[arg(long)]
        dest: Option<PathBuf>,
        /// Overwrite existing files at the destination(s).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SkillAgentArg {
    Claude,
    Codex,
    All,
}

impl From<SkillAgentArg> for crate::skill::AgentTarget {
    fn from(v: SkillAgentArg) -> Self {
        match v {
            SkillAgentArg::Claude => Self::Claude,
            SkillAgentArg::Codex => Self::Codex,
            SkillAgentArg::All => Self::All,
        }
    }
}

pub fn run() -> ExitCode {
    // `_log_guard` owns the non-blocking writer's worker thread. It MUST
    // stay alive for the whole of `run()`: dropping it flushes buffered
    // events and joins the thread, so binding it here keeps logs flowing
    // until every subcommand (including the long-lived `supervise` loop,
    // which exits its poll loop cooperatively on SIGINT/SIGTERM) has
    // returned. Caveat: subcommands that bypass unwinding via
    // `std::process::exit` (currently `event tail --follow`) skip this
    // drop and may lose this process's own buffered log events — tracked
    // as a follow-up, see issues/log-guard-flush-on-process-exit.
    let LoggingInit {
        warnings: logging_warnings,
        _guard: _log_guard,
    } = init_logging();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return handle_clap_error(e, &logging_warnings),
    };

    info!(
        target: "orchestratectl::cli",
        output_format = ?cli.output.format,
        output_file = ?cli.output.file,
        command = ?cli.command,
        "command dispatched"
    );

    let output = &cli.output;
    let result = match cli.command {
        Command::Version => cmd_version(output, &logging_warnings),
        Command::Skill { action } => match action {
            SkillAction::List => crate::skill::cmd_list(output, &logging_warnings),
            SkillAction::Show { name } => crate::skill::cmd_show(&name, output, &logging_warnings),
            SkillAction::Print { name } => {
                crate::skill::cmd_print(&name, output, &logging_warnings)
            }
            SkillAction::Install {
                name,
                agent,
                dest,
                force,
            } => crate::skill::cmd_install(
                name.as_deref(),
                agent.into(),
                dest,
                force,
                output,
                &logging_warnings,
            ),
        },
        Command::Run { action } => crate::run::dispatch(action, output, &logging_warnings),
        Command::Event { action } => crate::event::dispatch(action, output, &logging_warnings),
        Command::Node { action } => crate::node::dispatch(action, output, &logging_warnings),
        Command::Discussion { action } => {
            crate::discussion::dispatch(action, output, &logging_warnings)
        }
        Command::Spinoff { action } => crate::spinoff::dispatch(action, output, &logging_warnings),
        Command::Supervise(args) => crate::supervise::dispatch(args, output, &logging_warnings),
        // `doctor` owns its exit code directly: §18 requires exit 1 on any
        // `fail` *without* an error envelope (the report on stdout is the
        // answer), which does not map onto the shared `Result` path below.
        Command::Doctor(args) => return crate::doctor::run(&args, output, &logging_warnings),
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
        eprintln!("warning: {w}");
    }
    ExitCode::from(ExitKind::User as u8)
}

#[derive(Debug, Serialize)]
struct VersionPayload {
    version: &'static str,
    commit: &'static str,
    /// Bundled skill catalog (AGENTS-AI-FIRST-CLI §17). Each entry's
    /// `cli_version` is sourced from the embedded SKILL.md frontmatter,
    /// so an agent can audit "is the skill I loaded matching the binary
    /// I am about to call?" in one call.
    skills: Vec<crate::skill::SkillCatalogEntry>,
    // Duplicated from the success envelope intentionally. §10 of
    // AGENTS-AI-FIRST-CLI requires `version --json` to return
    // `{version, commit, schema_version, supported_schemas}` at the
    // payload level. Agents that unwrap `.data` must still see the
    // contract; omitting this field would make `.data.schema_version`
    // null while `.data.state_schema_version` is present — an
    // asymmetric foot-gun (review history/review-version-subcommand.md
    // §1).
    schema_version: u32,
    supported_schemas: &'static [u32],
    state_schema_version: u32,
    supported_state_schemas: &'static [u32],
}

fn cmd_version(spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let payload = VersionPayload {
        version: CARGO_VERSION,
        commit: GIT_COMMIT,
        skills: crate::skill::catalog(),
        schema_version: octl_core::SCHEMA_VERSION,
        supported_schemas: &[octl_core::SCHEMA_VERSION],
        state_schema_version: octl_core::STATE_SCHEMA_VERSION,
        supported_state_schemas: octl_core::SUPPORTED_STATE_SCHEMAS,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("orchestratectl {}", payload.version);
            println!("commit:                  {}", payload.commit);
            println!("envelope schema:         {}", payload.schema_version);
            println!(
                "supported envelopes:     {}",
                format_u32_list(payload.supported_schemas)
            );
            println!("state schema version:    {}", payload.state_schema_version);
            println!(
                "supported state schemas: {}",
                format_u32_list(payload.supported_state_schemas)
            );
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

fn parse_output_arg(s: &str) -> Result<OutputSpec, String> {
    output::parse_output_value(s)
}

fn format_u32_list(values: &[u32]) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Buffered-message capacity for the non-blocking log channel. Set
/// explicitly rather than relying on the crate default so the memory
/// ceiling (and the worst-case drain time on shutdown) is a deliberate,
/// visible choice. At the supervisor's 500ms × ~100-node cadence this is
/// far more headroom than a healthy disk needs.
const LOG_BUFFERED_LINES: usize = 128_000;

/// Result of [`init_logging`]: any non-fatal warnings to surface to the
/// caller, plus the worker guard whose lifetime keeps the background log
/// writer alive.
///
/// `#[must_use]`: the whole point of the struct is that `_guard` must be
/// bound for the lifetime of the process. Dropping it early flushes and
/// joins the writer thread, after which all further log events are
/// silently discarded.
#[must_use = "the log writer thread is shut down when the guard is dropped — bind it for the process lifetime"]
struct LoggingInit {
    warnings: Vec<String>,
    _guard: Option<WorkerGuard>,
}

/// Initialise the JSONL log subscriber. Logs go to
/// `~/.orchestratectl/logs/orchestratectl.log.jsonl`. Best-effort: if the
/// log file cannot be opened, the process still runs and the caller sees
/// the failure in the success-envelope `warnings` array or on stderr.
///
/// Returns the collected warnings plus the [`WorkerGuard`] for the
/// non-blocking writer. The guard owns the background writer thread; the
/// caller MUST keep it alive until the process is done logging, otherwise
/// buffered events are dropped on drop. The guard is `None` whenever the
/// subscriber was not installed (no log path, IO error, or an
/// already-initialised global subscriber).
///
/// Delivery semantics: the writer runs in **lossy** mode — if the channel
/// fills (a sustained burst the disk cannot keep up with) new events are
/// dropped rather than blocking the caller. This matches the MVP decision
/// to favour supervisor responsiveness over strict log completeness;
/// hardening (back-pressure / dropped-event accounting) is deferred. Logs
/// are also lost on `panic = "abort"` or a `std::process::exit` that
/// bypasses the guard's `Drop`.
fn init_logging() -> LoggingInit {
    let mut warnings = Vec::new();
    let log_path = match log_path() {
        Some(p) => p,
        None => {
            warnings
                .push("log path unavailable: HOME and ORCHESTRATECTL_HOME both unset".to_string());
            return LoggingInit {
                warnings,
                _guard: None,
            };
        }
    };

    if let Some(parent) = log_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            warnings.push(format!(
                "could not create log directory {}: {}",
                parent.display(),
                e
            ));
            return LoggingInit {
                warnings,
                _guard: None,
            };
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
            return LoggingInit {
                warnings,
                _guard: None,
            };
        }
    };

    let filter =
        EnvFilter::try_from_env("ORCHESTRATECTL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    // Hand the file to a background writer thread. The supervisor polls at
    // 500ms across ~100 nodes; doing the `write(2)` synchronously on the
    // tracing call path would serialise that hot loop on disk IO. The
    // single worker also serialises every record this process emits, so
    // no JSONL line is split or interleaved with another from *this*
    // process (cross-process appenders are still only protected at the
    // kernel's per-`write(2)` O_APPEND granularity). `lossy(true)` keeps
    // the tracing call path non-blocking under back-pressure; see the
    // delivery-semantics note above.
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(true)
        .buffered_lines_limit(LOG_BUFFERED_LINES)
        .finish(file);
    let layer = fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(writer);

    if let Err(e) = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
    {
        // Subscriber already installed (or some other install failure):
        // the layer we just built is unused, so let the guard drop here
        // (flushing and joining the idle worker) rather than handing back
        // a guard for a writer nobody reads.
        warnings.push(format!("tracing subscriber not installed: {e}"));
        return LoggingInit {
            warnings,
            _guard: None,
        };
    }

    LoggingInit {
        warnings,
        _guard: Some(guard),
    }
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
