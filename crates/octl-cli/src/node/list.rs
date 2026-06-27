//! `node list` — enumerate `nodes/*.json` for one run.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{read_manifest_opt, read_node_opt};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, kind_kebab, parse_node_id, run_paths, status_kebab};

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<String>,
    pub spec: &'a OutputSpec,
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
    let run_id = args.run_id.clone();
    if let Some(s) = &args.status {
        if s.trim().is_empty() {
            return Err(CliError::user(
                "invalid_value",
                "--status must not be empty",
            ));
        }
    }
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;
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
    let filter = args.status.as_deref().map(str::trim);
    if let Some(read) = entries {
        let mut ids: Vec<String> = Vec::new();
        for ent in read {
            // Propagate per-entry read errors. Silently swallowing
            // them (with `.filter_map(Result::ok)`) would hide
            // permission errors, broken NFS mounts, etc. and produce
            // a partial list that callers can't distinguish from a
            // truly empty run.
            let ent = ent.map_err(|e| {
                CliError::system(
                    "io_error",
                    format!("read_dir entry {}: {}", nodes_dir.display(), e),
                )
            })?;
            let p = ent.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
        ids.sort();
        for node_id in ids {
            // A directory entry whose stem is not a well-formed node id can't
            // be one of our projection files; skip it rather than erroring the
            // whole listing.
            let Ok(node_id) = parse_node_id(&node_id) else {
                continue;
            };
            let n = match read_node_opt(&paths, &node_id).map_err(from_core)? {
                Some(n) => n,
                None => continue,
            };
            // Use the canonical kebab-case helpers from `run/mod.rs`
            // rather than round-tripping the enum through
            // `serde_json::to_value`. Adding a new variant is then a
            // pattern-match compile error, not a silent `""`.
            let kind = kind_kebab(n.kind);
            let status = status_kebab(n.status);
            if let Some(f) = filter {
                if status != f {
                    continue;
                }
            }
            out.push(NodeSummary {
                node_id: n.node_id.to_string(),
                kind: kind.to_string(),
                status: status.to_string(),
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
    match args.spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, args.spec, args.warnings)?;
        }
        OutputFormat::Text => {
            if payload.nodes.is_empty() {
                println!("(no nodes)");
            }
            for n in &payload.nodes {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    n.node_id, n.kind, n.status, n.children, n.updated_at
                );
            }
            output::emit_text_warnings(args.warnings);
        }
    }
    Ok(())
}
