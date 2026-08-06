//! `run list` — walk `<root>/runs/` and emit a manifest summary per run.

use serde::Serialize;

use octl_core::{read_manifest_opt, read_node_opt, Kind, NodeId, RunLock, RunPaths, Status};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::dto::{RunSummary, SupervisorView};
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

    // One stall deadline per invocation — every run in this listing is judged
    // against the same instant, so two runs with identical `updated_at` can't
    // disagree on `stalled` due to per-run clock sampling.
    let now = chrono::Utc::now();

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
        // Each run carries its own `.lock`; take that run's shared lock for its
        // manifest read so the summary never reflects a manifest a reducer is
        // mid-rewrite on (design.md §4). A run with no `.lock` yet reads
        // lock-free (see `RunLock::acquire_shared`).
        // Read the manifest AND probe supervisor liveness under the SAME shared
        // lock so `status` and `supervisor` form one consistent snapshot — a
        // caller reasons "status pending + supervisor dead => orphaned", so the
        // pair must not straddle a reducer's status rollup + pid-file removal
        // (see show.rs). Costs one extra pid-file read per run; negligible for
        // realistic run counts.
        // Compute the `stalled` hint (issue `peculiarly-muddled-caption`) under
        // the SAME shared lock as the manifest so the node's
        // status/children/`updated_at` and the manifest form one consistent
        // snapshot that cannot straddle a reducer write. Read-only: touches no
        // event/reducer/schema path.
        //
        // The extra `n-0001` read is gated on `pending` + `orchestrate`: only a
        // pending orchestrate run can be a stall candidate, so terminal runs and
        // every other kind pay no extra I/O — and, critically, a corrupt/
        // unreadable `n-0001` in an unrelated run can no longer abort the whole
        // listing (the read only runs for the narrow candidate set).
        let scanned = RunLock::with_shared_lock(&paths.lock(), || {
            let Some(m) = read_manifest_opt(&paths)? else {
                return Ok(None);
            };
            let supervisor = SupervisorView::probe(&paths);
            let stalled = if m.status == Status::Pending && m.kind == Kind::Orchestrate {
                let driver_id = NodeId::parse_str(crate::run::stalled::DRIVER_NODE_ID)
                    .expect("DRIVER_NODE_ID is a valid node id");
                crate::run::stalled::is_stalled(
                    m.status,
                    m.kind,
                    read_node_opt(&paths, &driver_id)?.as_ref(),
                    now,
                )
            } else {
                false
            };
            Ok(Some((m, supervisor, stalled)))
        })
        .map_err(from_core)?;
        let (m, supervisor, stalled) = match scanned {
            Some(v) => v,
            None => continue, // half-initialized run dir; skip silently
        };
        // Shape the manifest into its wire DTO once, then filter on the
        // canonical kebab strings it carries — the DTO's `From` renders
        // `kind` / `status` through the `run/mod.rs` helpers rather than
        // round-tripping the enums through `serde_json::to_value`.
        let summary = RunSummary::from(&m)
            .with_supervisor(supervisor)
            .with_stalled(stalled);
        if let Some(filter) = &args.status {
            if &summary.status != filter {
                continue;
            }
        }
        if let Some(filter) = &args.kind {
            if &summary.kind != filter {
                continue;
            }
        }
        out.push(summary);
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
                let sup = match r.supervisor.pid {
                    Some(pid) if r.supervisor.alive => format!("sup:alive({pid})"),
                    Some(pid) => format!("sup:dead({pid})"),
                    None => "sup:none".to_string(),
                };
                // The status column carries a `stalled` marker for an undriven
                // orchestrate driver so a plain-text `run list` no longer shows
                // the zombie as an ordinary live `pending` run.
                let status = if r.stalled {
                    format!("{} (stalled)", r.status)
                } else {
                    r.status.clone()
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    r.run_id,
                    r.kind,
                    status,
                    r.node_count,
                    sup,
                    output::escape_one_line(&r.title)
                );
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
