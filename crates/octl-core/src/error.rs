//! Error type for `octl-core` file I/O and schema operations.

use std::path::PathBuf;

use crate::schema::Status;

/// Errors raised while reading, writing, or validating run state on disk.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A `cancel_run` was refused because the run is already in a *non-cancelled*
    /// terminal state (`Done` / `Failed`). Cancelling such a run would claim a
    /// transition the reducer's terminal-state guard refuses, so the operation
    /// is rejected up front without mutating any state. An already-`Cancelled`
    /// run is *not* this error — it converges (see [`crate::cancel_run`]).
    #[error("run is already terminal ({status:?}), cannot cancel")]
    RunAlreadyTerminal {
        /// The run's current terminal status (`Done` or `Failed`).
        status: Status,
    },

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

    /// A `node_id` handed to the event-append primitive failed validation
    /// before it could be written into an [`crate::schema::Event`] envelope.
    /// Callers are expected to pass an already-validated node id; this is the
    /// write-side guard that keeps an unvalidated id out of `events.jsonl`.
    #[error("invalid node_id {node_id:?}: {reason}")]
    InvalidNodeId {
        /// The rejected node id.
        node_id: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A projection file's embedded id contradicts where it lives on disk.
    ///
    /// Two distinct integrity faults share this variant, told apart by `kind`:
    ///
    /// - **Read side** (`kind` = `"node"` / `"discussion"` / `"spinoff"`): the
    ///   body's own id newtype is well-formed but does *not* equal the filename
    ///   key it was requested under — a valid `nodes/n-0002.json` placed at
    ///   `nodes/n-0001.json` deserializes fine yet describes a different node.
    ///   Returning it as `n-0002` would let a later `write_node` clobber a
    ///   third file, so the read is rejected instead.
    /// - **Write side** (`kind` = `"node_run_id"` / `"discussion_run_id"` /
    ///   `"spinoff_run_id"` / `"manifest_run_id"`): the object's `run_id` does
    ///   not equal the [`crate::paths::RunPaths`] run it would be written under,
    ///   so the write is refused before it can stamp a foreign run's id into
    ///   this run's directory.
    ///
    /// This is the projection-integrity guard, distinct from
    /// [`Error::CorruptEventLog`] (which guards `events.jsonl`). It is **not** a
    /// path-traversal vector — the keys are already validated id newtypes that
    /// cannot name a file outside the run directory — but a corruption /
    /// mis-placement detector. `kind` is a fixed `&'static str` so a caller can
    /// branch on it; `path` localizes the offending file; `expected_id` /
    /// `body_id` carry the two ids for an operator to diff.
    #[error(
        "corrupt projection ({kind}) at {path}: expected id {expected_id:?}, body has {body_id:?}"
    )]
    CorruptProjection {
        /// Which check fired: a read-side filename-key mismatch (`"node"`,
        /// `"discussion"`, `"spinoff"`) or a run-id mismatch (`"node_run_id"`,
        /// `"discussion_run_id"`, `"spinoff_run_id"`, `"manifest_run_id"`),
        /// which fires on both the read and write side — the fault is identical
        /// (the object's `run_id` does not equal its directory's run).
        kind: &'static str,
        /// The offending projection file (read side) or its intended
        /// destination (write side), so an operator can go straight to it.
        path: PathBuf,
        /// The id the file was expected to carry — the requested filename key
        /// (read-side key check) or the `RunPaths` run id (run-id check).
        expected_id: String,
        /// The id actually found in the file body.
        body_id: String,
    },

    /// The run directory itself is a symlink rather than a real directory.
    ///
    /// Best-effort symlink containment: [`crate::paths::RunPaths::new`] and every
    /// projection read/write reject a symlinked run root before any open follows
    /// it, so a replaced `<root>/runs/<id>` cannot redirect writes outside the
    /// run tree.
    ///
    /// **Trust model.** The state root is `$HOME/.orchestratectl/` — a per-user
    /// `0700` directory, not a shared multi-user mount. This guards against an
    /// accidentally- or maliciously-replaced subtree component, not a concurrent
    /// attacker who already holds write access to the state root.
    ///
    /// **Residual gap.** The check is check-then-open: a pure TOCTOU attacker can
    /// swap the path for a symlink in the window between the `symlink_metadata`
    /// call and the subsequent open. Closing that needs `O_NOFOLLOW` / `openat2`
    /// (`RESOLVE_BENEATH` / `RESOLVE_NO_SYMLINKS`), which the standard library
    /// does not expose portably; it is out of scope for the MVP threat model.
    #[error("run directory is a symlink (refusing to follow it): {path}")]
    SymlinkRunDir {
        /// The symlinked run directory.
        path: PathBuf,
    },

    /// A run subdirectory (`nodes/`, `discussions/`, `spinoffs/`) is a symlink.
    ///
    /// Same best-effort containment, trust model, and TOCTOU residual gap as
    /// [`Error::SymlinkRunDir`].
    #[error("run subdirectory {name:?} is a symlink (refusing to follow it): {path}")]
    SymlinkSubdir {
        /// The subdirectory name (`"nodes"`, `"discussions"`, `"spinoffs"`).
        name: &'static str,
        /// The symlinked subdirectory path.
        path: PathBuf,
    },

    /// A run-state file is a symlink rather than a regular file — covers the
    /// manifest, the event log, the lock file, and the per-id projection files
    /// (`name` discriminates: `"manifest"`, `"events"`, `"lock"`, `"node"`,
    /// `"discussion"`, `"spinoff"`).
    ///
    /// Same best-effort containment, trust model, and TOCTOU residual gap as
    /// [`Error::SymlinkRunDir`]. These files are created by the run itself
    /// (projection writes go via temp-file + rename, always regular files); a
    /// symlink in their place is a tampered or corrupted run.
    #[error("run state file {name:?} is a symlink (refusing to follow it): {path}")]
    SymlinkStateFile {
        /// Which state file (`"manifest"`, `"events"`, `"lock"`, `"node"`,
        /// `"discussion"`, `"spinoff"`).
        name: &'static str,
        /// The symlinked file path.
        path: PathBuf,
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
