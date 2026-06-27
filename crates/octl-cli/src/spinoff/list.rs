//! `spinoff list` — enumerate spin-off proposals for a run.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{read_manifest_opt, read_spinoff_opt};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, kind_kebab, require_safe_id, run_paths};
use crate::spinoff::{status_arg_kebab, status_kebab, StatusFilterArg};

pub struct Args<'a> {
    pub run_id: String,
    pub status: Option<StatusFilterArg>,
    pub spec: &'a OutputSpec,
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

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    // Validate inputs at the boundary *before* any filesystem work
    // (AGENTS-AI-FIRST-CLI §1: reject unknown values up front).
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let status_filter = args.status.map(status_arg_kebab);

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
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
            if let Some(filter) = status_filter {
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
    // Reverse-chronological with proposal_id as the tie-breaker so
    // same-millisecond proposals come out deterministically — matters
    // for AI consumers diffing list output.
    out.sort_by(|a, b| {
        b.proposed_at
            .cmp(&a.proposed_at)
            .then_with(|| a.proposal_id.cmp(&b.proposal_id))
    });

    emit(
        ListPayload {
            run_id,
            proposals: out,
        },
        args.spec,
        args.warnings,
    )
}

fn emit(payload: ListPayload, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            if payload.proposals.is_empty() {
                println!("(no proposals)");
            }
            for p in &payload.proposals {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    p.proposal_id, p.status, p.node_id, p.proposed_kind, p.proposed_title
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
