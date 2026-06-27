//! Core library for orchestratectl.
//!
//! See `issues/orchestratectl-mvp/design.md` for the canonical schema and
//! protocol references. This crate provides:
//!
//! - The on-disk schema types ([`Manifest`], [`Node`], [`Event`],
//!   [`Discussion`], [`SpinoffProposal`]).
//! - Atomic write helpers ([`atomic`]) and per-run advisory `flock`
//!   ([`RunLock`]).
//! - The event-append primitive ([`append_event`]) with `seq` recovery.
//! - The projection reducer ([`apply_event`]).
//!
//! Higher-level supervisor and CLI logic live in their own crates / issues.
//!
//! `octl-core` is the canonical library surface, so public items are required
//! to carry doc comments (`#![warn(missing_docs)]`). Lint-level policy
//! otherwise lives in the workspace `[workspace.lints]` table (pedantic clippy).
#![warn(missing_docs)]

pub mod atomic;
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

pub use envelope::SCHEMA_VERSION;
pub use error::{Error, Result};
pub use events::{
    append_and_apply, append_and_apply_unlocked, append_event, append_event_with_seq,
    read_all_events, recover_last_seq,
};
pub use ids::{format_node_id, new_discussion_id, new_proposal_id, new_run_id};
pub use lock::RunLock;
pub use paths::{run_dir, validate_run_id, RunPaths};
pub use projections::{
    read_discussion, read_discussion_opt, read_manifest, read_manifest_opt, read_node,
    read_node_opt, read_spinoff, read_spinoff_opt, write_discussion, write_manifest, write_node,
    write_spinoff,
};
pub use reducer::apply_event;
pub use report::{validate_report_payload, ReportValidationError};
pub use schema::{
    ChildRef, Discussion, DiscussionStatus, Event, Kind, Lifecycle, Manifest, Node,
    SpinoffProposal, SpinoffStatus, Status, STATE_SCHEMA_VERSION, SUPPORTED_STATE_SCHEMAS,
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
