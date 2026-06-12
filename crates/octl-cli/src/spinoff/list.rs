//! `spinoff list` — enumerate spin-off proposals for a run.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;

use octl_core::{read_manifest_opt, read_spinoff_opt};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, kind_kebab, require_safe_id, run_paths};
use crate::spinoff::status_kebab;

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<String>,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ListPayload {
    run_id: String,
    proposals: Vec<Summary>,
}

#[derive(Serialize)]
struct Summary {
    proposal_id: String,
    node_id: String,
    status: &'static str,
    proposed_title: String,
    proposed_kind: &'static str,
    proposed_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted_as_issue_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rejected_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_at: Option<DateTime<Utc>>,
}

const ALLOWED_FILTERS: &[&str] = &["pending", "approved", "rejected"];

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    if let Some(s) = args.status.as_deref() {
        if !ALLOWED_FILTERS.contains(&s) {
            return Err(CliError::user(
                "invalid_value",
                format!("--status must be one of {:?}", ALLOWED_FILTERS),
            )
            .with_invalid_value(s)
            .with_expected(json!(ALLOWED_FILTERS)));
        }
    }

    let mut out: Vec<Summary> = Vec::new();
    let dir = paths.spinoffs_dir();
    let read = match std::fs::read_dir(&dir) {
        Ok(e) => Some(e),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", dir.display(), e),
            ));
        }
    };
    if let Some(entries) = read {
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
        for id in ids {
            let s = match read_spinoff_opt(&paths, &id).map_err(from_core)? {
                Some(s) => s,
                None => continue,
            };
            let status = status_kebab(s.status);
            if let Some(filter) = args.status.as_deref() {
                if status != filter {
                    continue;
                }
            }
            out.push(Summary {
                proposal_id: s.proposal_id,
                node_id: s.node_id,
                status,
                proposed_title: s.proposed_title,
                proposed_kind: kind_kebab(s.proposed_kind),
                proposed_at: s.proposed_at,
                accepted_as_issue_slug: s.accepted_as_issue_slug,
                rejected_reason: s.rejected_reason,
                resolved_at: s.resolved_at,
            });
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.proposed_at));

    emit(
        ListPayload {
            run_id,
            proposals: out,
        },
        args.json,
        args.warnings,
    )
}

fn emit(payload: ListPayload, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        if payload.proposals.is_empty() {
            println!("(no proposals)");
        }
        for p in &payload.proposals {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                p.proposal_id, p.status, p.node_id, p.proposed_kind, p.proposed_title
            );
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
