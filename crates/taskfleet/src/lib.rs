//! The sole linkable Taskfleet command-line engine.
//!
//! The canonical binary and the bounded Cargo compatibility wrapper are
//! one-line adapters over [`dispatch`]. They select their invocation identity
//! explicitly; command behavior is never inferred from `argv[0]` or `PATH`.

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
mod state;
mod supervise;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Deprecated Cargo compatibility identity.
    pub const ORCHESTRATECTL: Self = Self {
        command_name: "orchestratectl",
    };

    /// Canonical Taskfleet identity.
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
    emit_compatibility_deprecation(identity);
    cli::run(identity)
}

static COMPATIBILITY_DEPRECATION_EMITTED: AtomicBool = AtomicBool::new(false);

fn emit_compatibility_deprecation(identity: InvocationIdentity) {
    if identity != InvocationIdentity::ORCHESTRATECTL
        || std::env::var_os(home::INTERNAL_SELF_EXEC_ENV).is_some()
        || COMPATIBILITY_DEPRECATION_EMITTED.swap(true, Ordering::Relaxed)
    {
        return;
    }
    eprintln!(
        "warning: `orchestratectl` is deprecated; use the canonical `taskfleet` command (the compatibility package is supported only through Taskfleet 0.7.x)"
    );
}
