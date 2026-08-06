//! `node list` — enumerate `nodes/*.json` for one run.

use serde::Serialize;

use octl_core::{read_manifest_opt, read_node_opt, RunLock};

use crate::error::CliError;
use crate::node::dto::NodeSummary;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, run_paths_from_cli_arg, status_kebab};

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
    let paths = run_paths_from_cli_arg(&root, &run_id)?;
    let filter = args.status.as_deref().map(str::trim);

    // Hold the run's shared lock across the manifest-existence check and the
    // whole `nodes/` scan so a concurrent reducer cannot add, remove, or rewrite
    // node projections mid-listing (design.md §4). `None` means the run is
    // missing (manifest absent); the lock is released before any output.
    let nodes_dir = paths.nodes_dir();
    let collected = RunLock::with_shared_lock(&paths.lock(), || {
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(None);
        }
        let mut out: Vec<NodeSummary> = Vec::new();
        let entries = match std::fs::read_dir(&nodes_dir) {
            Ok(e) => Some(e),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(octl_core::Error::io(&nodes_dir, e)),
        };
        if let Some(read) = entries {
            let mut ids: Vec<String> = Vec::new();
            for ent in read {
                // Propagate per-entry read errors. Silently swallowing
                // them (with `.filter_map(Result::ok)`) would hide
                // permission errors, broken NFS mounts, etc. and produce
                // a partial list that callers can't distinguish from a
                // truly empty run.
                let ent = ent.map_err(|e| octl_core::Error::io(&nodes_dir, e))?;
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
                let n = match read_node_opt(&paths, &node_id)? {
                    Some(n) => n,
                    None => continue,
                };
                // The `status` filter compares against the canonical
                // kebab string (`run/mod.rs` helper) rather than
                // round-tripping the enum through `serde_json::to_value`;
                // a new variant is then a pattern-match compile error, not
                // a silent `""`. Wire shaping happens in the DTO `From`.
                if let Some(f) = filter {
                    if status_kebab(n.status) != f {
                        continue;
                    }
                }
                out.push(NodeSummary::from(&n));
            }
        }
        Ok(Some(out))
    })
    .map_err(from_core)?;

    let Some(out) = collected else {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    };

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
