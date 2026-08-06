//! `discussion list` — walk `<run>/discussions/` and return a summary.

use serde::Serialize;

use octl_core::{read_discussion_opt, DiscussionStatus, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_discussion_id, run_paths_from_cli_arg};

use super::dto::DiscussionSummary;
use super::StatusArg;

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<StatusArg>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ListPayload {
    discussions: Vec<DiscussionSummary>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = &args.run_id;
    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, run_id)?;

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id.as_str()),
        );
    }

    let want = args.status.map(|s| match s {
        StatusArg::Open => DiscussionStatus::Open,
        StatusArg::Resolved => DiscussionStatus::Resolved,
    });

    // Hold the run's shared lock across the whole `discussions/` scan so a
    // concurrent reducer cannot add or rewrite a discussion mid-listing
    // (design.md §4). The lock is released before any output is formatted.
    let dir = paths.discussions_dir();
    let mut out: Vec<DiscussionSummary> = RunLock::with_shared_lock(&paths.lock(), || {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(octl_core::Error::io(&dir, e)),
        };

        let mut out: Vec<DiscussionSummary> = Vec::new();
        for ent in entries {
            let ent = ent.map_err(|e| octl_core::Error::io(&dir, e))?;
            // Skip directories, symlinks, FIFOs, sockets — `read_discussion_opt`
            // would surface them as opaque deserialization errors. Only
            // regular files are valid projection slots.
            if !ent.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let path = ent.path();
            let id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            // A stem that is not a well-formed discussion id can't be one of our
            // projection files; skip it rather than erroring the whole listing.
            let Ok(id) = parse_discussion_id(&id) else {
                continue;
            };
            let d = match read_discussion_opt(&paths, &id)? {
                Some(d) => d,
                None => continue,
            };
            if let Some(filter) = want {
                if d.status != filter {
                    continue;
                }
            }
            out.push(DiscussionSummary::from(&d));
        }
        Ok(out)
    })
    .map_err(from_core)?;

    // Newest first, with `discussion_id` as a stable secondary key so
    // two discussions opened in the same millisecond stay ordered
    // deterministically (`opened_at` resolution is RFC3339 ms).
    out.sort_by(|a, b| {
        b.opened_at
            .cmp(&a.opened_at)
            .then_with(|| a.discussion_id.cmp(&b.discussion_id))
    });
    emit(out, args.spec, args.warnings)
}

fn emit(
    discussions: Vec<DiscussionSummary>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&ListPayload { discussions }, spec, warnings)?;
        }
        OutputFormat::Text => {
            if discussions.is_empty() {
                println!("(no discussions)");
            }
            for d in &discussions {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    d.discussion_id,
                    d.node_id,
                    d.status,
                    output::escape_one_line(&d.severity),
                    output::escape_one_line(&d.topic)
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
