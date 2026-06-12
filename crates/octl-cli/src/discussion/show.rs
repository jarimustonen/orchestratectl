//! `discussion show` — full projection for one discussion.

use octl_core::{read_discussion_opt, Discussion};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

use super::status_kebab;

pub fn run(
    run_id: &str,
    discussion_id: &str,
    json: bool,
    warnings: &[String],
) -> Result<(), CliError> {
    let run_id = require_safe_id(run_id, "run-id")?;
    let discussion_id = require_safe_id(discussion_id, "discussion-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let disc = match read_discussion_opt(&paths, &discussion_id).map_err(from_core)? {
        Some(d) => d,
        None => {
            return Err(CliError::user(
                "discussion_not_found",
                format!("no discussion {discussion_id} in run {run_id}"),
            )
            .with_invalid_value(&discussion_id));
        }
    };

    emit(&disc, json, warnings)
}

fn emit(d: &Discussion, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(d, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("discussion-id: {}", d.discussion_id);
        println!("run-id:        {}", d.run_id);
        println!("node-id:       {}", d.node_id);
        println!("status:        {}", status_kebab(d.status));
        println!("severity:      {}", d.severity);
        println!("topic:         {}", d.topic);
        if let Some(c) = &d.context {
            println!("context:       {}", c);
        }
        if !d.options.is_empty() {
            println!("options:       {}", d.options.join(", "));
        }
        println!("opened_at:     {}", d.opened_at);
        if let Some(r) = &d.resolution {
            println!("resolution:    {}", r);
        }
        if let Some(n) = &d.note {
            println!("note:          {}", n);
        }
        if let Some(t) = d.resolved_at {
            println!("resolved_at:   {}", t);
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
