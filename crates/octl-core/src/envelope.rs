//! CLI envelope contract shared with downstream consumers.
//!
//! Per `AGENTS-AI-FIRST-CLI.md` §10, structured CLI output (success or
//! error) carries a `schema_version` field identifying the envelope
//! contract. Distinct from [`crate::STATE_SCHEMA_VERSION`], which
//! versions on-disk state files.

/// Current CLI envelope schema version.
pub const SCHEMA_VERSION: u32 = 1;
