//! `run cancel` — synthesize terminal `node.report` for non-terminal
//! nodes, emit `run.status: cancelled`. Idempotent re-cancel.

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{append_and_apply, read_manifest_opt, read_node_opt, Status};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, run_paths};

#[derive(Serialize)]
struct CancelPayload {
    run_id: String,
    cancelled_nodes: Vec<String>,
    already_cancelled: bool,
}

pub fn run(
    run_id: &str,
    note: Option<&str>,
    json: bool,
    warnings: &[String],
) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id);
    let manifest = match read_manifest_opt(&paths).map_err(from_core)? {
        Some(m) => m,
        None => {
            return Err(
                CliError::user("run-not-found", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
    };

    if matches!(manifest.status, Status::Cancelled) {
        return emit(
            CancelPayload {
                run_id: run_id.into(),
                cancelled_nodes: vec![],
                already_cancelled: true,
            },
            json,
            warnings,
        );
    }

    // Enumerate non-terminal nodes by scanning the nodes/ directory.
    // Reading each node JSON before deciding whether to synthesize keeps
    // us honest if some events were dropped and the directory listing
    // differs from manifest.node_count.
    let mut cancelled_nodes: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.nodes_dir()) {
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
            let n = match read_node_opt(&paths, &node_id).map_err(from_core)? {
                Some(n) => n,
                None => continue,
            };
            if matches!(n.status, Status::Done | Status::Failed | Status::Cancelled) {
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
            append_and_apply(&paths, "node.report", Some(&node_id), None, data)
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
            run_id: run_id.into(),
            cancelled_nodes,
            already_cancelled: false,
        },
        json,
        warnings,
    )
}

fn emit(payload: CancelPayload, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        if payload.already_cancelled {
            println!("run {} already cancelled (no-op)", payload.run_id);
        } else {
            println!(
                "cancelled run {} ({} node(s))",
                payload.run_id,
                payload.cancelled_nodes.len()
            );
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
