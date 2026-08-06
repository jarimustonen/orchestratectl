//! `discussion show` — full projection for one discussion.

use octl_core::{read_discussion_opt, Discussion};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_discussion_id, run_paths_from_cli_arg};

use super::dto::DiscussionView;
use super::status_kebab;

pub fn run(
    run_id: &str,
    discussion_id: &str,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let discussion_id = parse_discussion_id(discussion_id)?;
    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, run_id)?;

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id),
        );
    }

    let disc = match read_discussion_opt(&paths, &discussion_id).map_err(from_core)? {
        Some(d) => d,
        None => {
            return Err(CliError::user(
                "discussion_not_found",
                format!("no discussion {discussion_id} in run {run_id}"),
            )
            .with_invalid_value(discussion_id.as_str()));
        }
    };

    emit(&disc, spec, warnings)
}

fn emit(d: &Discussion, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&DiscussionView::from(d), spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("discussion-id: {}", d.discussion_id);
            println!("run-id:        {}", d.run_id);
            println!("node-id:       {}", d.node_id);
            println!("status:        {}", status_kebab(d.status));
            println!("severity:      {}", output::escape_one_line(&d.severity));
            println!("topic:         {}", output::escape_one_line(&d.topic));
            if let Some(c) = &d.context {
                println!("context:       {}", output::escape_one_line(c));
            }
            if !d.options.is_empty() {
                let opts = d
                    .options
                    .iter()
                    .map(|o| output::escape_one_line(o))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("options:       {opts}");
            }
            println!("opened_at:     {}", d.opened_at);
            if let Some(r) = &d.resolution {
                println!("resolution:    {}", output::escape_one_line(r));
            }
            if let Some(n) = &d.note {
                println!("note:          {}", output::escape_one_line(n));
            }
            if let Some(t) = d.resolved_at {
                println!("resolved_at:   {t}");
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
