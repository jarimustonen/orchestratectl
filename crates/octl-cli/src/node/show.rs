//! `node show` — full `nodes/<node-id>.json` projection.

use octl_core::{read_manifest_opt, read_node_opt, RunLock};

use crate::error::CliError;
use crate::node::dto::NodeView;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, run_paths, status_kebab};

pub fn run(
    run_id: &str,
    node_id: &str,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let node_id = parse_node_id(node_id)?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id)?;
    // The run-existence check and the node read are two file reads; hold the
    // shared lock across both so a concurrent reducer cannot delete or rewrite
    // the node between them (design.md §4). `None` distinguishes a missing run
    // (manifest absent) from a missing node — resolved into errors after the
    // lock is released.
    let outcome = RunLock::with_shared_lock(&paths.lock(), || {
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(None);
        }
        Ok(Some(read_node_opt(&paths, &node_id)?))
    })
    .map_err(from_core)?;
    let node = match outcome {
        None => {
            return Err(
                CliError::user("run_not_found", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
        Some(None) => {
            return Err(CliError::user(
                "node_not_found",
                format!("no node {node_id} in run {run_id}"),
            )
            .with_invalid_value(node_id.as_str()));
        }
        Some(Some(n)) => n,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&NodeView::from(&node), spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:        {}", node.run_id);
            println!("node-id:       {}", node.node_id);
            println!("status:        {}", status_kebab(node.status));
            if let Some(p) = &node.parent_node_id {
                println!("parent-node:   {p}");
            }
            if let Some(t) = &node.task {
                println!("task:          {}", output::escape_one_line(t));
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
