//! `spinoff` subcommand — list/approve/reject spin-off proposals.
//!
//! Mirrors the noun-module pattern set by `run` / `event`: one file per
//! verb, shared types in `mod.rs`, single `dispatch` entry point called
//! from `cli.rs`.

pub mod approve;
pub mod list;
pub mod reject;

use clap::Subcommand;

use crate::error::CliError;

#[derive(Subcommand, Debug)]
pub enum SpinoffAction {
    /// List spin-off proposals for a run.
    List {
        run_id: String,
        /// Filter by status (`pending`, `approved`, `rejected`).
        ///
        /// `pending` is the human/CLI synonym for the on-disk
        /// `proposed` status (design.md §1.5 names the schema variant
        /// `Proposed`, but every callsite — including the CLI flag in
        /// design.md §2.5 — uses `pending`).
        #[arg(long)]
        status: Option<String>,
    },
    /// Approve a proposal: emit `spinoff.approved` and (optionally)
    /// materialize an issue via `issuectl new`.
    Approve {
        run_id: String,
        proposal_id: String,
        /// Caller asserts the issue is already materialized at this
        /// slug; skips the `issuectl new` call.
        #[arg(long)]
        issue_slug: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Reject a proposal: emit `spinoff.rejected`.
    Reject {
        run_id: String,
        proposal_id: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn dispatch(action: SpinoffAction, json: bool, warnings: &[String]) -> Result<(), CliError> {
    match action {
        SpinoffAction::List { run_id, status } => list::run(list::Args {
            run_id,
            status,
            json,
            warnings,
        }),
        SpinoffAction::Approve {
            run_id,
            proposal_id,
            issue_slug,
            idempotency_key,
            dry_run,
        } => approve::run(approve::Args {
            run_id,
            proposal_id,
            issue_slug,
            idempotency_key,
            dry_run,
            json,
            warnings,
        }),
        SpinoffAction::Reject {
            run_id,
            proposal_id,
            reason,
            idempotency_key,
            dry_run,
        } => reject::run(reject::Args {
            run_id,
            proposal_id,
            reason,
            idempotency_key,
            dry_run,
            json,
            warnings,
        }),
    }
}

/// Map an on-disk `SpinoffStatus` to the kebab string used in CLI
/// output and `--status` filtering. `proposed` becomes `pending` so the
/// CLI surface matches design.md §2.5 verbiage.
pub fn status_kebab(s: octl_core::SpinoffStatus) -> &'static str {
    use octl_core::SpinoffStatus::*;
    match s {
        Proposed => "pending",
        Approved => "approved",
        Rejected => "rejected",
    }
}
