//! `run list` — walk `<root>/runs/` and emit a manifest summary per run.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{read_manifest_opt, RunPaths};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, runs_root};

pub struct Args<'a> {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ListPayload {
    runs: Vec<RunSummary>,
}

#[derive(Serialize)]
struct RunSummary {
    run_id: String,
    kind: String,
    status: String,
    title: String,
    created_at: DateTime<Utc>,
    node_count: u32,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let runs_dir = runs_root(&root);

    // Strict-input rule from AGENTS-AI-FIRST-CLI §1: only validate that
    // the filter values are well-formed strings. We don't reject unknown
    // kinds/statuses up front because a filter that matches nothing is a
    // legitimate empty result — different from a malformed value.
    if let Some(s) = &args.status {
        if s.trim().is_empty() {
            return Err(CliError::user(
                "invalid_value",
                "--status must not be empty",
            ));
        }
    }
    if let Some(k) = &args.kind {
        if k.trim().is_empty() {
            return Err(CliError::user("invalid_value", "--kind must not be empty"));
        }
    }

    let mut out: Vec<RunSummary> = Vec::new();
    let entries = match std::fs::read_dir(&runs_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return emit(out, args.spec, args.warnings);
        }
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", runs_dir.display(), e),
            ));
        }
    };

    for ent in entries {
        let ent = ent.map_err(|e| CliError::system("io_error", e.to_string()))?;
        if !ent.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        // The directory name must be a valid run id; foreign dirs are skipped.
        let Some(run_id) = ent.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(paths) = RunPaths::new(ent.path(), run_id) else {
            continue;
        };
        let m = match read_manifest_opt(&paths).map_err(from_core)? {
            Some(m) => m,
            None => continue, // half-initialized run dir; skip silently
        };
        let kind = serde_json::to_value(m.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let status = serde_json::to_value(m.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        if let Some(filter) = &args.status {
            if &status != filter {
                continue;
            }
        }
        if let Some(filter) = &args.kind {
            if &kind != filter {
                continue;
            }
        }
        out.push(RunSummary {
            run_id: m.run_id,
            kind,
            status,
            title: m.title,
            created_at: m.created_at,
            node_count: m.node_count,
        });
    }

    out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    emit(out, args.spec, args.warnings)
}

fn emit(runs: Vec<RunSummary>, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&ListPayload { runs }, spec, warnings)?;
        }
        OutputFormat::Text => {
            if runs.is_empty() {
                println!("(no runs)");
            }
            for r in &runs {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    r.run_id, r.kind, r.status, r.node_count, r.title
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
