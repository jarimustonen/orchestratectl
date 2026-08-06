//! `spinoff list` — enumerate spin-off proposals for a run.

use serde::Serialize;

use octl_core::{read_manifest_opt, read_spinoff_opt, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_proposal_id, run_paths_from_cli_arg};
use crate::spinoff::dto::SpinoffSummary;
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
    proposals: Vec<SpinoffSummary>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    // Validate inputs at the boundary *before* any filesystem work
    // (AGENTS-AI-FIRST-CLI §1: reject unknown values up front).
    let run_id = &args.run_id;
    let status_filter = args.status.map(status_arg_kebab);

    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, run_id)?;

    // Hold the run's shared lock across the manifest-existence check and the
    // whole `spinoffs/` scan so a concurrent reducer cannot add or rewrite a
    // proposal mid-listing (design.md §4). `None` means the run is missing
    // (manifest absent); the lock is released before any output.
    let dir = paths.spinoffs_dir();
    let collected = RunLock::with_shared_lock(&paths.lock(), || {
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(None);
        }
        let mut out: Vec<SpinoffSummary> = Vec::new();
        let read = match std::fs::read_dir(&dir) {
            Ok(e) => Some(e),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(octl_core::Error::io(&dir, e)),
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
                // A stem that is not a well-formed proposal id can't be one of our
                // projection files; skip it rather than erroring the whole listing.
                let Ok(id) = parse_proposal_id(&id) else {
                    continue;
                };
                let s = match read_spinoff_opt(&paths, &id)? {
                    Some(s) => s,
                    None => continue,
                };
                if let Some(filter) = status_filter {
                    if status_kebab(s.status) != filter {
                        continue;
                    }
                }
                out.push(SpinoffSummary::from(&s));
            }
        }
        Ok(Some(out))
    })
    .map_err(from_core)?;

    let Some(mut out) = collected else {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id.as_str()),
        );
    };
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
            run_id: run_id.clone(),
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
                    p.proposal_id,
                    p.status,
                    p.node_id,
                    p.proposed_kind,
                    output::escape_one_line(&p.proposed_title)
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
