//! The light worker-harness launcher (0.2 harness surface).
//!
//! `run create --harness <name>` picks which agent runtime launches the worker
//! in its tmux pane. The mechanism is deliberately narrow: a resolved harness
//! name maps to a **workmux agent** ([`workmux_agent`]) forwarded to `create.sh`
//! as `--agent <name>`; the supervisor / merge / report path is
//! harness-agnostic, so every harness rides the same lifecycle.
//!
//! - [`select`] — the `flag > env > config-file > built-in default` precedence
//!   resolver for `--harness`.
//! - [`prompt`] — the pi worker-prompt translation shim (Claude-Code-flavored
//!   briefs → pi/CLI equivalents for the autonomous kinds pi supports).
//! - [`KNOWN_HARNESSES`] / [`DEFAULT_HARNESS`] / [`workmux_agent`] — the
//!   registry, the default, and the name→agent mapping.
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

/// Map a resolved harness name to the workmux agent to launch in the worker's
/// tmux pane (`create.sh --agent` → `workmux add -a`).
///
/// `claude` maps to `None`, preserving its legacy workmux-default launch shape:
/// no `--agent` is passed. `pi`, including the built-in default, maps to an
/// explicit `--agent pi`. **Every other name** — including one this build does
/// not recognise — forwards verbatim as the workmux agent (so `--harness pi`
/// runs `workmux add -a pi`; workmux must have that agent configured).
///
/// The unknown case forwards rather than falling back to `None` **on purpose**:
/// the supervisor's retry path reads `manifest.harness` and passes it here, and
/// a value written by a newer build (or hand-edited) must NEVER silently
/// re-launch a run under claude. Forwarding an unknown agent lets workmux fail
/// loudly instead — the "retry never silently drops back to claude" guarantee.
/// The `run create` path validates against [`KNOWN_HARNESSES`] before it ever
/// reaches here, so a fresh run's known name is the common case.
#[must_use]
pub fn workmux_agent(harness: &str) -> Option<&str> {
    if harness == "claude" {
        None
    } else {
        Some(harness)
    }
}
