//! `spinoff` subcommand — list/approve/reject spin-off proposals.
//!
//! Mirrors the noun-module pattern set by `run` / `event`: one file per
//! verb, shared types in `mod.rs`, single `dispatch` entry point called
//! from `cli.rs`.

pub mod approve;
pub mod list;
pub mod reject;

use clap::{Subcommand, ValueEnum};

use crate::error::CliError;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub enum StatusFilterArg {
    /// `Proposed` on disk — the noun-level vocabulary surfaces
    /// `pending` to match design.md §2.5.
    Pending,
    Approved,
    Rejected,
}

#[derive(Subcommand, Debug)]
pub enum SpinoffAction {
    /// List spin-off proposals for a run.
    List {
        run_id: String,
        /// Filter by status (`pending`, `approved`, `rejected`).
        #[arg(long, value_enum)]
        status: Option<StatusFilterArg>,
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

pub fn status_arg_kebab(s: StatusFilterArg) -> &'static str {
    match s {
        StatusFilterArg::Pending => "pending",
        StatusFilterArg::Approved => "approved",
        StatusFilterArg::Rejected => "rejected",
    }
}

/// Maximum byte length for `--issue-slug` and `--reason` values.
/// Bounds the projection / event-log row size and keeps human output
/// readable. 128 chars is enough for a kebab-case slug; 1024 is
/// enough for a one-sentence rejection reason.
const MAX_SLUG_BYTES: usize = 128;
const MAX_REASON_BYTES: usize = 1024;

/// Reject empty/oversize slugs and any character outside
/// `[a-z0-9-]`. The slug ends up in the event log and may later be
/// used as part of a filesystem path or shell token by downstream
/// tooling; validating at the CLI boundary keeps the rest of the
/// system honest.
pub fn require_safe_slug(value: &str, field: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must not be empty or whitespace-only"),
        )
        .with_invalid_value(value));
    }
    if trimmed.len() > MAX_SLUG_BYTES {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must be at most {MAX_SLUG_BYTES} bytes"),
        )
        .with_invalid_value(value));
    }
    let charset_ok = trimmed
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !charset_ok || trimmed.starts_with('-') || trimmed.ends_with('-') {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must be lowercase kebab-case ([a-z0-9-], no leading/trailing `-`)"),
        )
        .with_invalid_value(value));
    }
    Ok(trimmed.to_string())
}

/// Validate a free-text `--reason` (or any similar short prose field).
/// Strips leading/trailing whitespace, rejects empty values, caps
/// length, and rejects control characters (other than tab) that would
/// corrupt human output and downstream log consumers.
pub fn validate_reason_like(value: &str, field: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must not be empty or whitespace-only"),
        )
        .with_invalid_value(value));
    }
    if trimmed.len() > MAX_REASON_BYTES {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must be at most {MAX_REASON_BYTES} bytes"),
        )
        .with_invalid_value(value));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\t' && c != '\n')
    {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must not contain control characters"),
        )
        .with_invalid_value(value));
    }
    Ok(trimmed.to_string())
}
