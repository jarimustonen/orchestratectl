//! Error type for `octl-core` file I/O and schema operations.

use std::path::PathBuf;

/// Errors raised while reading, writing, or validating run state on disk.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem I/O failure with the offending path attached for context.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path the operation was acting on when it failed.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// I/O failure with no path context (from `?` on a bare `io::Error`).
    #[error("io error: {0}")]
    IoBare(#[from] std::io::Error),

    /// JSON (de)serialization failure with the offending path attached.
    #[error("json error at {path}: {source}")]
    Json {
        /// Path of the JSON document being parsed or written.
        path: PathBuf,
        /// Underlying `serde_json` error.
        #[source]
        source: serde_json::Error,
    },

    /// JSON failure with no path context (from `?` on a bare `serde_json::Error`).
    #[error("json error: {0}")]
    JsonBare(#[from] serde_json::Error),

    /// An `events.jsonl` line could not be parsed or violated an invariant.
    #[error("corrupt event log at {path}: {reason}")]
    CorruptEventLog {
        /// Path to the event log.
        path: PathBuf,
        /// Human-readable description of what was malformed.
        reason: String,
    },

    /// A `run_id` failed validation when constructing [`crate::paths::RunPaths`].
    #[error("invalid run_id {run_id:?}: {reason}")]
    InvalidRunId {
        /// The rejected run id.
        run_id: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A state file declared a `schema_version` this build does not support.
    #[error("invalid schema_version {found} (supported: {supported:?}) at {path}")]
    UnsupportedSchemaVersion {
        /// Path to the offending state file.
        path: PathBuf,
        /// The `schema_version` value read from disk.
        found: u32,
        /// Versions this build can read (see [`SUPPORTED_STATE_SCHEMAS`]).
        ///
        /// [`SUPPORTED_STATE_SCHEMAS`]: crate::schema::SUPPORTED_STATE_SCHEMAS
        supported: Vec<u32>,
    },
}

/// Convenience alias for results returned by `octl-core`.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Construct an [`Error::Io`] tagging `source` with the `path` it failed on.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Construct an [`Error::Json`] tagging `source` with the `path` it failed on.
    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}
