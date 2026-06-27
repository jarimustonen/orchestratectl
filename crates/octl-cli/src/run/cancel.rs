//! `run cancel` — synthesize terminal `node.report` for non-terminal
//! nodes, emit `run.status: cancelled`. Idempotent re-cancel.

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{append_and_apply, read_manifest_opt, read_node_opt, Status};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, run_paths};

#[derive(Serialize)]
struct CancelPayload {
    run_id: String,
    cancelled_nodes: Vec<String>,
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
    let manifest = match read_manifest_opt(&paths).map_err(from_core)? {
        Some(m) => m,
        None => {
            return Err(
                CliError::user("run_not_found", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
    };

    if matches!(manifest.status, Status::Cancelled) {
        return emit(
            CancelPayload {
                run_id: run_id.to_string(),
                cancelled_nodes: vec![],
                already_cancelled: true,
            },
            spec,
            warnings,
        );
    }

    // Enumerate non-terminal nodes by scanning the nodes/ directory.
    // Reading each node JSON before deciding whether to synthesize keeps
    // us honest if some events were dropped and the directory listing
    // differs from manifest.node_count.
    let mut cancelled_nodes: Vec<String> = Vec::new();
    let nodes_dir = paths.nodes_dir();
    let read = match std::fs::read_dir(&nodes_dir) {
        Ok(e) => Some(e),
        // No nodes/ yet (run never reached create.sh) — nothing to cancel.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", nodes_dir.display(), e),
            ));
        }
    };
    if let Some(entries) = read {
        let mut ids: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") {
                    return None;
                }
                p.file_stem().and_then(|s| s.to_str()).map(str::to_string)
            })
            .collect();
        ids.sort();
        for node_id in ids {
            // A stem that is not a well-formed node id can't be one of our
            // projection files; skip it rather than failing the cancel.
            let Ok(nid) = parse_node_id(&node_id) else {
                continue;
            };
            let n = match read_node_opt(&paths, &nid).map_err(from_core)? {
                Some(n) => n,
                None => continue,
            };
            // Skip already-settled nodes: synthesizing a cancel report for
            // them would be a no-op anyway (the reducer's terminal-state
            // guard rejects transitions out of a terminal state), so this
            // just avoids appending a dead event. The reducer remains the
            // canonical gate.
            if n.status.is_terminal() {
                continue;
            }
            let reason = note.unwrap_or("cancelled by user");
            let data: Value = json!({
                "success": false,
                "cancelled": true,
                "reason": reason,
                "summary": "Run cancelled before agent reported.",
                "discussion_items": [],
                "spinoff_proposals": [],
                "wrap_up_recommendations": []
            });
            append_and_apply(&paths, "node.report", Some(nid.as_str()), None, data)
                .map_err(from_core)?;
            cancelled_nodes.push(node_id);
        }
    }

    let mut status_data = serde_json::Map::new();
    status_data.insert("status".into(), Value::String("cancelled".into()));
    if let Some(n) = note {
        status_data.insert("note".into(), Value::String(n.into()));
    }
    append_and_apply(&paths, "run.status", None, None, Value::Object(status_data))
        .map_err(from_core)?;

    emit(
        CancelPayload {
            run_id: run_id.to_string(),
            cancelled_nodes,
            already_cancelled: false,
        },
        spec,
        warnings,
    )
}

fn emit(payload: CancelPayload, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            if payload.already_cancelled {
                println!("run {} already cancelled (no-op)", payload.run_id);
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
