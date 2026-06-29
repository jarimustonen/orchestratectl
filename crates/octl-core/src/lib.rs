//! Core library for orchestratectl.
//!
//! See `issues/orchestratectl-mvp/design.md` for the canonical schema and
//! protocol references. This crate provides:
//!
//! - The on-disk schema types ([`Manifest`], [`Node`], [`Event`],
//!   [`Discussion`], [`SpinoffProposal`]).
//! - Atomic write helpers ([`atomic`]) and per-run advisory `flock`
//!   ([`RunLock`]).
//! - The canonical mutation entry point
//!   ([`append_and_apply_event`]): append one
//!   event and fold it into the projections under the run's `flock`.
//!
//! Higher-level supervisor and CLI logic live in their own crates / issues.
//!
//! `octl-core` is the canonical library surface, so public items are required
//! to carry doc comments (`#![warn(missing_docs)]`). Lint-level policy
//! otherwise lives in the workspace `[workspace.lints]` table (pedantic clippy).
#![warn(missing_docs)]

pub mod atomic;
pub mod cancel;
pub mod envelope;
pub mod error;
pub mod events;
pub mod ids;
pub mod lock;
pub mod paths;
pub mod projections;
pub mod reducer;
pub mod report;
pub mod schema;

#[cfg(test)]
mod stress_tests;

pub use cancel::{cancel_run, cancel_run_unlocked, CancelOutcome};
pub use envelope::SCHEMA_VERSION;
pub use error::{Error, Result};
pub use events::{
    append_and_apply_event, append_and_apply_unlocked, quarantine_corrupt_lines,
    quarantine_corrupt_lines_unlocked, read_all_events, recover_last_seq, AppendResult, PriorEvent,
    Quarantine,
};
pub use ids::{format_node_id, new_discussion_id, new_proposal_id, new_run_id};
pub use lock::{Exclusive, LockedRun, RunLock, Shared};
pub use paths::{nofollow, run_dir, validate_run_id, RunPaths};
pub use projections::{
    read_discussion, read_discussion_opt, read_manifest, read_manifest_opt, read_node,
    read_node_opt, read_spinoff, read_spinoff_opt, write_node,
};
pub use report::{validate_report_payload, ReportValidationError};
pub use schema::{
    ChildRef, Discussion, DiscussionId, DiscussionStatus, Event, IdValidationError, Kind,
    Lifecycle, Manifest, Node, NodeId, ProposalId, RunId, SpinoffProposal, SpinoffStatus, Status,
    STATE_SCHEMA_VERSION, SUPPORTED_STATE_SCHEMAS,
};

/// Ensure the orchestratectl root directory exists (`<root>/runs`,
/// `<root>/logs`). Idempotent.
pub fn ensure_root(root: &std::path::Path) -> Result<()> {
    for sub in ["runs", "logs"] {
        let p = root.join(sub);
        std::fs::create_dir_all(&p).map_err(|e| Error::io(&p, e))?;
    }
    Ok(())
}
