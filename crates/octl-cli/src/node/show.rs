//! `node show` — full `nodes/<node-id>.json` projection.

use octl_core::{read_manifest_opt, read_node_opt};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, require_safe_id, run_paths, status_kebab};

pub fn run(
    run_id: &str,
    node_id: &str,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let run_id = require_safe_id(run_id, "run-id")?;
    let node_id = require_safe_id(node_id, "node-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }
    let node = match read_node_opt(&paths, &node_id).map_err(from_core)? {
        Some(n) => n,
        None => {
            return Err(CliError::user(
                "node_not_found",
                format!("no node {node_id} in run {run_id}"),
            )
            .with_invalid_value(&node_id));
        }
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&node, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:        {}", node.run_id);
            println!("node-id:       {}", node.node_id);
            println!("status:        {}", status_kebab(node.status));
            if let Some(p) = &node.parent_node_id {
                println!("parent-node:   {}", p);
            }
            if let Some(t) = &node.task {
                println!("task:          {}", t);
            }
            println!("children:      {}", node.children.len());
            println!("updated_at:    {}", node.updated_at);
            if node.last_report.is_some() {
                println!("last_report:   (present)");
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
