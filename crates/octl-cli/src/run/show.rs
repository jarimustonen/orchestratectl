//! `run show` — full manifest + counters for one run.

use serde::Serialize;

use octl_core::{read_manifest_opt, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::dto::{ManifestView, SupervisorView};
use crate::run::{from_core, run_paths};

#[derive(Serialize)]
struct ShowPayload<'a> {
    manifest: ManifestView<'a>,
    counts: Counts,
}

#[derive(Serialize)]
struct Counts {
    nodes: u64,
    discussions: u64,
    spinoffs: u64,
}

pub fn run(run_id: &str, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id)?;
    // Hold the run's shared lock for the whole manifest + projection-dir scan so
    // a concurrent reducer cannot leave us with a manifest counter that
    // disagrees with the projection files we count (design.md §4). The lock is
    // released before any output is formatted.
    let scanned = RunLock::with_shared_lock(&paths.lock(), || {
        let Some(manifest) = read_manifest_opt(&paths)? else {
            return Ok(None);
        };
        let counts = Counts {
            nodes: count_jsons(&paths.nodes_dir()),
            discussions: count_jsons(&paths.discussions_dir()),
            spinoffs: count_jsons(&paths.spinoffs_dir()),
        };
        // Probe supervisor liveness INSIDE the shared lock: the whole point of
        // the field is to let a caller reason "status pending + supervisor dead
        // => orphaned", so `manifest.status` and `supervisor` are a single
        // decision and must be read as one consistent snapshot (design.md §4).
        // Reading the pid file outside the lock could emit a
        // `{status: pending, supervisor: dead}` pair that never actually existed
        // (the supervisor may roll status up and remove its pid file between the
        // two reads).
        let supervisor = SupervisorView::probe(&paths);
        Ok(Some((manifest, counts, supervisor)))
    })
    .map_err(from_core)?;
    let (manifest, counts, supervisor) = match scanned {
        Some(v) => v,
        None => {
            return Err(
                CliError::user("run_not_found", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
    };
    let payload = ShowPayload {
        manifest: ManifestView::from(&manifest).with_supervisor(supervisor),
        counts,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:        {}", payload.manifest.run_id);
            println!(
                "title:         {}",
                output::escape_one_line(payload.manifest.title)
            );
            println!("status:        {}", payload.manifest.status);
            println!("kind:          {}", payload.manifest.kind);
            println!("lifecycle:     {}", payload.manifest.lifecycle);
            println!("created_at:    {}", payload.manifest.created_at);
            println!("updated_at:    {}", payload.manifest.updated_at);
            println!("nodes:         {}", payload.counts.nodes);
            println!("discussions:   {}", payload.counts.discussions);
            println!("spinoffs:      {}", payload.counts.spinoffs);
            match payload.manifest.supervisor.pid {
                Some(pid) if payload.manifest.supervisor.alive => {
                    println!("supervisor:    pid {pid} (alive)");
                }
                Some(pid) => println!(
                    "supervisor:    pid {pid} (dead — run `orchestratectl run reattach {}` to recover)",
                    payload.manifest.run_id
                ),
                None => println!("supervisor:    (none recorded)"),
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

fn count_jsons(dir: &std::path::Path) -> u64 {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        })
        .count() as u64
}
