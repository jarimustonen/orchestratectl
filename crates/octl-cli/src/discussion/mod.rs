//! `discussion` subcommand — read (`list`, `show`) and resolve (`resolve`)
//! discussion projections under `<root>/runs/<run-id>/discussions/`.
//!
//! Verbs follow the noun-module pattern set by `run` and `event`:
//! one file per verb, shared types + dispatch in `mod.rs`.

pub mod list;
pub mod resolve;
pub mod show;

use clap::{Subcommand, ValueEnum};

use crate::error::CliError;
use crate::output::OutputSpec;

/// Status filter for `discussion list`.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub enum StatusArg {
    Open,
    Resolved,
}

#[derive(Subcommand, Debug)]
pub enum DiscussionAction {
    /// List discussions for a run.
    List {
        run_id: String,
        /// Filter by status (`open` or `resolved`). When omitted, all are listed.
        #[arg(long, value_enum)]
        status: Option<StatusArg>,
    },
    /// Show a single discussion projection.
    Show {
        run_id: String,
        discussion_id: String,
    },
    /// Resolve a discussion — emits `discussion.resolved`, updates the
    /// projection. Idempotent on `--choice`; conflict on `--choice` change.
    Resolve {
        run_id: String,
        discussion_id: String,
        /// Resolution choice (free-form string from the agent operator).
        #[arg(long)]
        choice: String,
        /// Optional human-readable note recorded with the resolution.
        #[arg(long)]
        note: Option<String>,
        /// Dedup token for retries (e.g. supervisor crash + replay).
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Print the would-be event and exit 0 without touching the
        /// filesystem.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn dispatch(
    action: DiscussionAction,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match action {
        DiscussionAction::List { run_id, status } => list::run(list::Args {
            run_id,
            status,
            spec,
            warnings,
        }),
        DiscussionAction::Show {
            run_id,
            discussion_id,
        } => show::run(&run_id, &discussion_id, spec, warnings),
        DiscussionAction::Resolve {
            run_id,
            discussion_id,
            choice,
            note,
            idempotency_key,
            dry_run,
        } => resolve::run(resolve::Args {
            run_id,
            discussion_id,
            choice,
            note,
            idempotency_key,
            dry_run,
            spec,
            warnings,
        }),
    }
}

pub fn status_kebab(s: octl_core::DiscussionStatus) -> &'static str {
    use octl_core::DiscussionStatus::{Open, Resolved};
    match s {
        Open => "open",
        Resolved => "resolved",
    }
}
