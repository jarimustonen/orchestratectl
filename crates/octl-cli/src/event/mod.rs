//! `event` subcommand — read/write events on the canonical event log.
//!
//! Verbs (one file per verb, shared types here):
//!
//! - `tail`   — read (`tail.rs`, design.md §2.3).
//! - `create` — sanctioned write path (`create.rs`, design.md §1, §2.3).
//!   Direct `echo ... >> events.jsonl` is explicitly banned because macOS
//!   lacks `flock(1)` and portable shell-side locking can't be enforced;
//!   `event create` acquires the per-run `flock`, assigns `seq`, appends,
//!   runs the reducer, fsyncs, and releases — all in one atomic window.
//!
//! `dispatch` is the single entry point called from `cli.rs`.

pub mod create;
pub mod tail;

use std::path::PathBuf;

use clap::Subcommand;

use crate::error::CliError;
use crate::output::{OutputFormat, OutputSpec};

/// Output format for streaming verbs. `Text` is the human-readable
/// per-event line; `Jsonl` is the canonical machine stream (one JSON
/// event per line). Pretty `Json` is not a valid stream format and the
/// caller is rejected with `unsupported_format` if they pass it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatArg {
    Text,
    Jsonl,
}

/// Map the global `OutputFormat` to the streaming `FormatArg`. Single
/// JSON document mode is rejected — a pretty-printed multi-line JSON
/// payload is neither a valid single document (the stream is open-ended)
/// nor valid JSONL (one-per-line). Streaming verbs must declare format
/// up front per AGENTS-AI-FIRST-CLI §12.
pub fn resolve_format(fmt: OutputFormat) -> Result<FormatArg, CliError> {
    match fmt {
        OutputFormat::Text => Ok(FormatArg::Text),
        OutputFormat::Jsonl => Ok(FormatArg::Jsonl),
        OutputFormat::Json => Err(CliError::user(
            "unsupported_format",
            "streaming verbs do not support --output json (pretty single-document); use jsonl or text",
        )),
    }
}

#[derive(Subcommand, Debug)]
pub enum EventAction {
    /// Stream events from a run's `events.jsonl`. Tails to EOF and exits
    /// (or follows the file with `--follow`, polling every 500ms).
    ///
    /// JSONL mode emits one object per line and terminates with exactly
    /// one `{"event":"result"|"cancelled","schema_version":1, ...}` envelope.
    /// With `--follow`, SIGINT exits 130. SIGTERM also exits 130 (known
    /// divergence from AGENTS-AI-FIRST-CLI §12's 143 — `ctrlc` does not
    /// surface the signal value; see comment in `tail.rs`).
    Tail {
        run_id: String,
        /// Emit only events with `seq >= from_seq` (default 0 = all).
        #[arg(long, default_value_t = 0)]
        from_seq: u64,
        /// Keep polling for new events after reaching EOF.
        #[arg(long)]
        follow: bool,
        /// Write the stream to this file instead of stdout. Truncated on
        /// open without `--follow`, append-mode with `--follow`. Must not
        /// point at the run's own `events.jsonl`. The format is the
        /// global `--output` selection (text or jsonl).
        #[arg(long)]
        to_file: Option<std::path::PathBuf>,
    },
    /// Append one event to a run's `events.jsonl` and update projections.
    Create {
        /// Target run-id (`<root>/runs/<run-id>` must exist).
        run_id: String,
        /// Event kind (see design.md §1.4 for the closed MVP set).
        #[arg(long)]
        kind: String,
        /// Node-id for node-scoped kinds (`node.*`, `child.spawned`).
        #[arg(long)]
        node_id: Option<String>,
        /// JSON file containing the event's `data` payload.
        #[arg(long)]
        from_file: PathBuf,
        /// Dedup token — a repeat call with the same key returns the
        /// existing event's `seq` instead of appending again.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Print the would-be event + projection plan and exit 0 without
        /// touching the filesystem.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn dispatch(action: EventAction, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match action {
        EventAction::Tail {
            run_id,
            from_seq,
            follow,
            to_file,
        } => tail::run(tail::Args {
            run_id,
            from_seq,
            follow,
            to_file,
            spec,
            warnings,
        }),
        EventAction::Create {
            run_id,
            kind,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
        } => create::run(create::Args {
            run_id,
            kind,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
            spec,
            warnings,
        }),
    }
}
