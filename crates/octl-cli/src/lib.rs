//! Linkable command-line engine for orchestratectl.
//!
//! The binary is intentionally a one-line adapter over [`dispatch`]. Future
//! canonical and compatibility binaries link this same crate and select their
//! invocation identity explicitly; command behavior is never inferred from
//! `argv[0]` or a `PATH` lookup.

mod cli;
// User-facing configuration file (`~/.orchestratectl/config.toml`), the `file`
// layer of the flag > env > file > default precedence (AGENTS-AI-FIRST-CLI §8).
mod config;
mod doctor;
mod error;
mod event;
mod git;
mod harness;
mod help;
mod home;
mod idempotency;
mod multiplexer;
mod node;
mod output;
mod proc;
mod run;
mod run_worker;
mod self_exec;
mod skill;
mod supervise;

use std::process::ExitCode;

/// Explicit branding for one binary entry point into the shared CLI engine.
///
/// R1 intentionally exposes only the current identity. Later rename phases can
/// add the canonical identity and bounded deprecation metadata here without
/// changing parser or command execution ownership. The identity is supplied by
/// the binary at compile time; the engine never guesses it from `argv[0]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct InvocationIdentity {
    command_name: &'static str,
}

impl InvocationIdentity {
    /// Current orchestratectl invocation identity.
    pub const ORCHESTRATECTL: Self = Self {
        command_name: "orchestratectl",
    };

    /// Future canonical Taskfleet identity. No shipped binary selects this in
    /// R1, so adding the link-time seam is behavior-neutral.
    pub const TASKFLEET: Self = Self {
        command_name: "taskfleet",
    };

    pub(crate) const fn command_name(self) -> &'static str {
        self.command_name
    }
}

/// Dispatch the current process through the sole shared parser/execution engine.
///
/// This is a process entry point: it reads the current argv/environment, writes
/// to process stdout/stderr, and initializes process-global logging once.
pub fn dispatch(identity: InvocationIdentity) -> ExitCode {
    cli::run(identity)
}
