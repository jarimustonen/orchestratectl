//! The light worker-harness launcher (0.2 harness surface).
//!
//! `run create --harness <name>` picks which agent runtime launches the worker
//! in its tmux pane. The mechanism is deliberately narrow: a resolved harness
//! name resolves to an exact candidate argv. Native materialization passes a
//! private generated launcher to `workmux add -a`; the supervisor / merge /
//! report path remains harness-agnostic.
//!
//! - [`select`] — the `flag > env > config-file > built-in default` precedence
//!   resolver for `--harness`.
//! - [`prompt`] — the pi worker-prompt translation shim (Claude-Code-flavored
//!   briefs → pi/CLI equivalents for the autonomous kinds pi supports).
//! - [`KNOWN_HARNESSES`] / [`DEFAULT_HARNESS`] — the built-in registry and
//!   default used when no executable profile is configured.
//!
//! The heavy in-process `CodeHarness` layer (the code-pipeline bakeoff /
//! conformance suite and the aider / claude-deepseek adapters) was cut in the
//! 0.2 simplification; only the launcher survives.

pub mod profile;
pub mod prompt;
pub mod select;
// Test-only shared machinery (a process-wide env lock). Gated at the declaration
// so a release build carries no empty module.
#[cfg(test)]
pub(crate) mod support;

/// The canonical set of harness names, in registry order. Single source of truth
/// for a valid `run create --harness <name>` value.
pub const KNOWN_HARNESSES: &[&str] = &["claude", "pi"];

/// The built-in default harness when no flag / env / config selects one. Per ADR
/// 0001 D4, pi.dev is the universal default; claude is a non-default opt-in.
pub const DEFAULT_HARNESS: &str = "pi";
