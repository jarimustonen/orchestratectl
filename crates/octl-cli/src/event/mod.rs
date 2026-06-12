//! `event` subcommand — read/write events on the canonical event log.
//!
//! Expected verbs (one file per verb, shared types here):
//!
//! - `tail`   — read (this module's `tail.rs`, design.md §2.3).
//! - `create` — append (parallel `event-create-cli` spinoff — keep this
//!   module's surface compatible but do **not** edit `create.rs` here).
//!
//! `dispatch` is the single entry point called from `cli.rs`.

pub mod tail;

use clap::{Subcommand, ValueEnum};

use crate::error::CliError;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum FormatArg {
    Text,
    Json,
    Jsonl,
}

#[derive(Subcommand, Debug)]
pub enum EventAction {
    /// Stream events from a run's `events.jsonl`. Tails to EOF and exits
    /// (or follows the file with `--follow`, polling every 500ms).
    Tail {
        run_id: String,
        /// Emit only events with `seq >= from_seq` (default 0 = all).
        #[arg(long, default_value_t = 0)]
        from_seq: u64,
        /// Keep polling for new events after reaching EOF.
        #[arg(long)]
        follow: bool,
        /// Output format: `text` (human), `json` (pretty), `jsonl` (one-line).
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// Write to file instead of stdout. Truncated on open without
        /// `--follow`, append-mode with `--follow`.
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
}

pub fn dispatch(action: EventAction, json: bool, warnings: &[String]) -> Result<(), CliError> {
    match action {
        EventAction::Tail {
            run_id,
            from_seq,
            follow,
            format,
            output,
        } => tail::run(tail::Args {
            run_id,
            from_seq,
            follow,
            format,
            output,
            json,
            warnings,
        }),
    }
}
