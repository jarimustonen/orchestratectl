//! Core library for orchestratectl.
//!
//! Schema types, file I/O, locking, and supervisor protocol land in
//! subsequent issues (`state-schema-crate`, etc.). This crate currently
//! only carries the canonical schema version constants the CLI binary
//! reads.

/// The state-on-disk schema version this binary writes.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// All state schema versions this binary can read.
pub const SUPPORTED_STATE_SCHEMAS: &[u32] = &[1];
