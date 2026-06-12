//! `node list` — enumerate `nodes/*.json` for one run.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{read_manifest_opt, read_node_opt};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<String>,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ListPayload {
    run_id: String,
    nodes: Vec<NodeSummary>,
}

#[derive(Serialize)]
struct NodeSummary {
    node_id: String,
    kind: String,
    status: String,
    parent_node_id: Option<String>,
    updated_at: DateTime<Utc>,
    children: u32,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    if let Some(s) = &args.status {
        if s.trim().is_empty() {
            return Err(CliError::user(
                "invalid_value",
                "--status must not be empty",
            ));
        }
    }
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let nodes_dir = paths.nodes_dir();
    let mut out: Vec<NodeSummary> = Vec::new();
    let entries = match std::fs::read_dir(&nodes_dir) {
        Ok(e) => Some(e),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", nodes_dir.display(), e),
            ));
        }
    };
    if let Some(read) = entries {
        let mut ids: Vec<String> = read
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
            let kind = serde_json::to_value(n.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let status = serde_json::to_value(n.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            if let Some(filter) = &args.status {
                if &status != filter {
                    continue;
                }
            }
            out.push(NodeSummary {
                node_id: n.node_id,
                kind,
                status,
                parent_node_id: n.parent_node_id,
                updated_at: n.updated_at,
                children: n.children.len() as u32,
            });
        }
    }

    let payload = ListPayload {
        run_id: run_id.clone(),
        nodes: out,
    };
    if args.json {
        output::emit_json(&payload, args.warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        if payload.nodes.is_empty() {
            println!("(no nodes)");
        }
        for n in &payload.nodes {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                n.node_id, n.kind, n.status, n.children, n.updated_at
            );
        }
        for w in args.warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
