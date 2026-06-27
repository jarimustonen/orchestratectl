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
use crate::run::{from_core, run_paths, status_kebab};

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
    let paths = run_paths(&root, run_id)?;

    // A missing manifest is a definitive "no such run" — surface the friendly
    // `run_not_found` (exit 1) before taking the lock. core::cancel_run re-reads
    // the manifest under the lock, so this pre-check is purely for the nicer
    // error class; a run that races into existence afterward still cancels
    // correctly.
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id),
        );
    }

    let outcome = match cancel_run(&paths, note) {
        Ok(o) => o,
        // A Done/Failed run can't be cancelled: refusing here (exit 2) is
        // honest where the old path appended events the reducer then dropped
        // while still printing success.
        Err(octl_core::Error::RunAlreadyTerminal { status }) => {
            let s = status_kebab(status);
            return Err(CliError::system(
                "run_already_terminal",
                format!("run is {s}, cannot cancel"),
            )
            .with_invalid_value(s)
            .with_expected(json!("running|pending|blocked")));
        }
        Err(e) => return Err(from_core(e)),
    };

    emit(run_id, &outcome, spec, warnings)
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
            if payload.already_cancelled {
                println!(
                    "no-op: run {} was already cancelled, converged {} additional node(s)",
                    payload.run_id,
                    payload.cancelled_nodes.len()
                );
            } else {
                println!(
                    "cancelled run {} ({} node(s))",
                    payload.run_id,
                    payload.cancelled_nodes.len()
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
