//! `run` subcommand — top-level lifecycle for orchestratectl runs.
//!
//! Sets the noun-module pattern that `node`, `event`, `discussion`,
//! `spinoff` will follow: one file per verb, shared types in `mod.rs`,
//! single `dispatch` entry point called from `cli.rs`.

pub mod cancel;
pub mod create;
pub mod list;
pub mod reattach;
pub mod show;

use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use octl_core::{Kind, Lifecycle, RunPaths};

use crate::error::CliError;

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum KindArg {
    Code,
    Spinoff,
    Orchestrated,
    Research,
    TechnicalDecision,
    MakeSkill,
    FanOut,
    Bugfix,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Code => Kind::Code,
            KindArg::Spinoff => Kind::Spinoff,
            KindArg::Orchestrated => Kind::Orchestrated,
            KindArg::Research => Kind::Research,
            KindArg::TechnicalDecision => Kind::TechnicalDecision,
            KindArg::MakeSkill => Kind::MakeSkill,
            KindArg::FanOut => Kind::FanOut,
            KindArg::Bugfix => Kind::Bugfix,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum RunAction {
    /// Create a new run. Top-level when `--parent-*` flags are absent,
    /// child-spawn when both are set (mutually required).
    Create {
        #[arg(long, value_enum)]
        kind: KindArg,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_repo: Option<String>,
        #[arg(long)]
        source_branch: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long, requires = "parent_node_id")]
        parent_run_id: Option<String>,
        #[arg(long, requires = "parent_run_id")]
        parent_node_id: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// List runs on disk.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show one run's manifest and counters.
    Show { run_id: String },
    /// Cancel a run: synthesize terminal `node.report` for non-terminal
    /// nodes, emit `run.status: cancelled`. Idempotent.
    Cancel {
        run_id: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Request supervisor reattach. Until `supervisor-process` lands,
    /// this only records the request event for later replay.
    Reattach { run_id: String },
}

pub fn dispatch(action: RunAction, json: bool, warnings: &[String]) -> Result<(), CliError> {
    match action {
        RunAction::Create {
            kind,
            title,
            source_repo,
            source_branch,
            task,
            parent_run_id,
            parent_node_id,
            idempotency_key,
            dry_run,
        } => create::run(create::Args {
            kind: kind.into(),
            title,
            source_repo,
            source_branch,
            task,
            parent_run_id,
            parent_node_id,
            idempotency_key,
            dry_run,
            json,
            warnings,
        }),
        RunAction::List { status, kind } => list::run(list::Args {
            status,
            kind,
            json,
            warnings,
        }),
        RunAction::Show { run_id } => show::run(&run_id, json, warnings),
        RunAction::Cancel { run_id, note } => cancel::run(&run_id, note.as_deref(), json, warnings),
        RunAction::Reattach { run_id } => reattach::run(&run_id, json, warnings),
    }
}

/// Map a `Kind` to its default `Lifecycle` per design.md §7.4.
pub fn lifecycle_for(k: Kind) -> Lifecycle {
    match k {
        Kind::Code => Lifecycle::Interactive,
        _ => Lifecycle::Autonomous,
    }
}

/// Resolve `<root>/runs/<run-id>` and return `RunPaths`.
pub fn run_paths(root: &Path, run_id: &str) -> RunPaths {
    RunPaths::new(octl_core::run_dir(root, run_id))
}

/// Trim a CLI string argument and reject empty/whitespace-only values.
pub fn require_nonempty(value: &str, field: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must not be empty or whitespace-only"),
        )
        .with_invalid_value(value));
    }
    Ok(trimmed.to_string())
}

/// Reject identifier strings that could escape the runs/ directory.
///
/// The run-id is user-controlled at the `show`/`cancel`/`reattach` call
/// sites and at `--parent-run-id`. Without validation, values like
/// `../../etc` would let an attacker walk outside `<root>/runs/`.
/// Accepts the ULID charset our own generator emits plus `n-` style
/// node ids: ASCII alphanumeric plus `-` and `_`.
pub fn require_safe_id(value: &str, field: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::user(
            "invalid_id",
            format!("--{field} must be ASCII alphanumeric + `-`/`_` and not `.`/`..`"),
        )
        .with_invalid_value(value));
    }
    Ok(trimmed.to_string())
}

/// Render a `Kind` as its canonical kebab-case wire string. Single
/// source of truth shared by every verb so create/list/show/json/text
/// stay aligned and adding a new kind only requires editing here.
pub fn kind_kebab(k: Kind) -> &'static str {
    match k {
        Kind::Code => "code",
        Kind::Spinoff => "spinoff",
        Kind::Orchestrated => "orchestrated",
        Kind::Research => "research",
        Kind::TechnicalDecision => "technical-decision",
        Kind::MakeSkill => "make-skill",
        Kind::FanOut => "fan-out",
        Kind::Bugfix => "bugfix",
    }
}

pub fn lifecycle_kebab(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Autonomous => "autonomous",
        Lifecycle::Interactive => "interactive",
    }
}

pub fn status_kebab(s: octl_core::Status) -> &'static str {
    use octl_core::Status::*;
    match s {
        Pending => "pending",
        Running => "running",
        Blocked => "blocked",
        Done => "done",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

/// `<root>/runs/`.
pub fn runs_root(root: &Path) -> PathBuf {
    root.join("runs")
}

/// Map a core::Error into a CliError.
pub fn from_core(err: octl_core::Error) -> CliError {
    CliError::system("io_error", err.to_string())
}
