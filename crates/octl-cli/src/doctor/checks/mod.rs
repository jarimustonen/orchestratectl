//! The MVP check set (AGENTS-AI-FIRST-CLI §18).
//!
//! Six categories, each in its own module, each emitting one
//! [`CheckResult`] per finding (never
//! summarized into one):
//!
//! - [`binary`] — `binary.commit`: build commit and applicable repo HEAD drift.
//! - [`schema`] — `schema.runs.<id>`: every run manifest deserializes.
//! - [`skill`] — `skill.sync.<name>`: on-disk skill `cli_version` matches.
//! - [`deps`] — `dep.<bin>`: each shelled-out binary is on `PATH`.
//! - [`config`] — `config.home`: the orchestratectl home resolves.
//! - [`data`] — `data.orphan-supervisor.<id>`: no dead supervisor PIDs.

pub mod binary;
pub mod config;
pub mod data;
pub mod deps;
pub mod schema;
pub mod skill;

use std::path::PathBuf;

use super::check::CheckResult;

/// Read-only context handed to every check. `root` is the resolved
/// orchestratectl home (`$ORCHESTRATECTL_HOME` or `~/.orchestratectl`);
/// it is `None` only when neither `$ORCHESTRATECTL_HOME` nor `$HOME` is
/// set, in which case the root-dependent checks degrade gracefully and
/// `config.home` reports the failure.
pub struct Ctx {
    pub root: Option<PathBuf>,
}

/// Run every check category in a deterministic order and return the flat
/// list of findings. Order is category-then-discovery so the text/jsonl
/// stream reads top-down (binary, config, deps, schema, skill, data).
pub fn run_all(ctx: &Ctx) -> Vec<CheckResult> {
    let mut out = Vec::new();
    out.extend(binary::check(ctx));
    out.extend(config::check(ctx));
    out.extend(deps::check(ctx));
    out.extend(schema::check(ctx));
    out.extend(skill::check(ctx));
    out.extend(data::check(ctx));
    out
}
