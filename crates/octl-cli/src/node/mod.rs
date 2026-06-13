//! `node` subcommand — read (`list`, `show`) + agent self-report (`report`).
//!
//! Mirrors the noun-module pattern set by `run/`: one file per verb,
//! shared types here, single `dispatch` entry point called from
//! `cli.rs`. The `report` verb is a domain verb (design.md §2.0, §2.2)
//! — it's the agent's structured-report sink and accepts the §7.3
//! payload schema directly rather than going through `event create`.

pub mod list;
pub mod report;
pub mod show;

use std::path::PathBuf;

use clap::Subcommand;

use crate::error::CliError;
use crate::output::OutputSpec;

#[derive(Subcommand, Debug)]
pub enum NodeAction {
    /// List nodes belonging to a run.
    List {
        run_id: String,
        /// Filter by status (e.g. `running`, `done`).
        #[arg(long)]
        status: Option<String>,
    },
    /// Print one node's JSON projection.
    Show { run_id: String, node_id: String },
    /// Agent self-submission of a structured terminal report (§7.3).
    Report {
        run_id: String,
        node_id: String,
        /// JSON file containing the §7.3 report payload.
        #[arg(long)]
        from_file: PathBuf,
        /// Dedup token — a repeat call with the same key returns the
        /// existing event's `seq` instead of appending again.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Validate the payload and exit 0 without writing anything
        /// to the run's events.jsonl or projection files.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn dispatch(
    action: NodeAction,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match action {
        NodeAction::List { run_id, status } => list::run(list::Args {
            run_id,
            status,
            spec,
            warnings,
        }),
        NodeAction::Show { run_id, node_id } => show::run(&run_id, &node_id, spec, warnings),
        NodeAction::Report {
            run_id,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
        } => report::run(report::Args {
            run_id,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
            spec,
            warnings,
        }),
    }
}
