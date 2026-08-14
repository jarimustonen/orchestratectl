//! `run cancel` — thin wrapper over [`octl_core::cancel_run`].
//!
//! All the cancel semantics (single-lock transaction, terminal-run refusal,
//! convergent re-cancel, honest node accounting) live in core. This file just
//! resolves the run, translates the [`octl_core::CancelOutcome`] /
//! [`octl_core::Error::RunAlreadyTerminal`] into the CLI's envelope, and prints.

use serde::Serialize;
use serde_json::json;

use octl_core::{cancel_run, read_manifest_opt, CancelOutcome, NodeId};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, run_paths_from_cli_arg, status_kebab};

#[derive(Serialize)]
struct CancelPayload {
    run_id: String,
    cancelled_nodes: Vec<String>,
    nodes_already_terminal: Vec<String>,
    already_cancelled: bool,
}

pub fn run(
    run_id: &str,
    note: Option<&str>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, run_id)?;
    // `run_id` may have been an unambiguous prefix; from here on report the full
    // resolved id so payloads/messages never echo a partial id back to the user.
    let run_id = paths.run_id.as_str();

    // Friendly `run_not_found` (exit 1) for a missing manifest, BEFORE taking
    // the lock — and, importantly, before `cancel_run` calls `RunLock::acquire`,
    // which would create `<run-dir>/.lock` (and its parent) as a side effect for
    // a bogus run-id. This pre-check is best-effort, not authoritative: if the
    // run is deleted in the window between here and the lock, `cancel_run`'s own
    // manifest read fails with a NotFound I/O error, which the match below maps
    // back to the same `run_not_found` envelope so the race can't leak an
    // `io_error`.
    match read_manifest_opt(&paths).map_err(from_core)? {
        None => return Err(run_not_found(run_id)),
        // A run recorded under a removed kind is read-only (ADR §D7) — refuse
        // before `cancel_run` appends its cancel events and rewrites the
        // manifest (which would overwrite the legacy kind with `"unknown"`).
        Some(m) => crate::run::reject_legacy_kind(m.kind, paths.run_id.as_str())?,
    }

    let outcome = match cancel_run(&paths, note) {
        Ok(o) => o,
        // A Done/Failed run can't be cancelled: refusing here is honest where
        // the old path appended events the reducer then dropped while still
        // printing success.
        Err(octl_core::Error::RunAlreadyTerminal { status }) => {
            let s = status_kebab(status);
            return Err(CliError::user(
                "run_already_terminal",
                format!("run is {s}, cannot cancel"),
            )
            .with_invalid_value(s)
            .with_expected(json!(["running", "pending", "blocked"])));
        }
        // The run vanished between the pre-check and the lock: report it as the
        // same missing-run condition rather than a generic system I/O error.
        Err(octl_core::Error::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Err(run_not_found(run_id));
        }
        Err(e) => return Err(from_core(e)),
    };

    emit(run_id, &outcome, spec, warnings)
}

fn run_not_found(run_id: &str) -> CliError {
    CliError::user("run_not_found", format!("no run with id {run_id}")).with_invalid_value(run_id)
}

fn emit(
    run_id: &str,
    outcome: &CancelOutcome,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let payload = CancelPayload {
        run_id: run_id.to_string(),
        cancelled_nodes: outcome.nodes_cancelled.iter().map(node_str).collect(),
        nodes_already_terminal: outcome
            .nodes_already_terminal
            .iter()
            .map(node_str)
            .collect(),
        already_cancelled: outcome.run_was_already_cancelled,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            let cancelled = payload.cancelled_nodes.len();
            let already = payload.nodes_already_terminal.len();
            if payload.already_cancelled {
                println!(
                    "no-op: run {} was already cancelled, converged {cancelled} additional node(s) ({already} already terminal)",
                    payload.run_id,
                );
            } else {
                println!(
                    "cancelled run {} ({cancelled} node(s) cancelled, {already} already terminal)",
                    payload.run_id,
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

fn node_str(id: &NodeId) -> String {
    id.as_str().to_string()
}
