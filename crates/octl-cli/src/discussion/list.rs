//! `discussion list` — walk `<run>/discussions/` and return a summary.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{read_discussion_opt, DiscussionStatus};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

use super::{status_kebab, StatusArg};

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<StatusArg>,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ListPayload {
    discussions: Vec<DiscussionSummary>,
}

#[derive(Serialize)]
struct DiscussionSummary {
    discussion_id: String,
    node_id: String,
    status: String,
    severity: String,
    topic: String,
    opened_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_at: Option<DateTime<Utc>>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let want = args.status.map(|s| match s {
        StatusArg::Open => DiscussionStatus::Open,
        StatusArg::Resolved => DiscussionStatus::Resolved,
    });

    let dir = paths.discussions_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return emit(Vec::new(), args.json, args.warnings);
        }
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", dir.display(), e),
            ));
        }
    };

    let mut out: Vec<DiscussionSummary> = Vec::new();
    for ent in entries {
        let ent = ent.map_err(|e| CliError::system("io_error", e.to_string()))?;
        let path = ent.path();
        let id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| !s.eq_ignore_ascii_case("json"))
            .unwrap_or(true)
        {
            continue;
        }
        let d = match read_discussion_opt(&paths, &id).map_err(from_core)? {
            Some(d) => d,
            None => continue,
        };
        if let Some(filter) = want {
            if d.status != filter {
                continue;
            }
        }
        out.push(DiscussionSummary {
            discussion_id: d.discussion_id,
            node_id: d.node_id,
            status: status_kebab(d.status).to_string(),
            severity: d.severity,
            topic: d.topic,
            opened_at: d.opened_at,
            resolved_at: d.resolved_at,
        });
    }

    out.sort_by_key(|d| std::cmp::Reverse(d.opened_at));
    emit(out, args.json, args.warnings)
}

fn emit(
    discussions: Vec<DiscussionSummary>,
    json: bool,
    warnings: &[String],
) -> Result<(), CliError> {
    if json {
        output::emit_json(&ListPayload { discussions }, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        if discussions.is_empty() {
            println!("(no discussions)");
        }
        for d in &discussions {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                d.discussion_id, d.node_id, d.status, d.severity, d.topic
            );
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
