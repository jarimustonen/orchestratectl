//! `run list` — walk `<root>/runs/` and emit a manifest summary per run.

use serde::Serialize;

use octl_core::{read_manifest_opt, RunLock, RunPaths};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::dto::{RunSummary, SupervisorState, SupervisorView};
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

/// Minimum age (from `manifest.created_at`) a zero-node, no-supervisor run must
/// reach before `run list` flags it `stillborn`.
///
/// `run list` sweeps EVERY run, including ones another process is mid-`run
/// create` on. For a top-level worker the supervisor is spawned only AFTER
/// `node.created`, so during the whole `create.sh` window (up to
/// `--agent-startup-timeout`, capped at 600s) a perfectly healthy in-flight run
/// transiently presents the exact stillborn shape — pending, 0 nodes, no
/// supervisor pid, `updated_at == created_at`. Gating the flag on an age that
/// comfortably exceeds the max create window keeps a bulk `run list` (e.g. a
/// monitor over `--json`, or the parallel-spawn wave in the incident that filed
/// this) from flagging — or a script from cancelling — a run that is simply
/// still being created.
///
/// `run show` / `run wait` deliberately need NO such gate: they are invoked on a
/// *specific* run whose `run create` already returned, where 0 nodes means
/// create.sh failed (definitively stillborn) — and `run wait` MUST settle
/// promptly, so [`is_stillborn`](crate::run::stalled::is_stillborn) itself stays
/// grace-free (issue `run-wait-stillborn-run-not-detected`, which a grace would
/// re-break). The gate lives here, at the one read surface that sweeps in-flight
/// creates. Matches the supervisor's own no-worker grace (900s). Overridable via
/// [`STILLBORN_LIST_GRACE_ENV`] (tests set `0` to flag immediately).
const STILLBORN_LIST_GRACE_SECS: i64 = 900;

/// Env override for [`STILLBORN_LIST_GRACE_SECS`] (whole seconds; unparseable →
/// default). Tests set `0` to flag a freshly-created stillborn run immediately.
const STILLBORN_LIST_GRACE_ENV: &str = "OCTL_STILLBORN_LIST_GRACE_SECS";

/// The effective stillborn grace, honoring [`STILLBORN_LIST_GRACE_ENV`].
fn stillborn_list_grace() -> chrono::Duration {
    let secs = std::env::var(STILLBORN_LIST_GRACE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(STILLBORN_LIST_GRACE_SECS);
    chrono::Duration::seconds(secs)
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
    // Minimum age a run must reach before `run list` flags it `stillborn`.
    let stillborn_grace = stillborn_list_grace();

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
        // Compute the `stillborn` hint under the SAME shared lock as the manifest
        // so the manifest and the pid probe form one consistent snapshot that
        // cannot straddle a reducer write. Read-only: touches no
        // event/reducer/schema path.
        let scanned = RunLock::with_shared_lock(&paths.lock(), || {
            let Some(m) = read_manifest_opt(&paths)? else {
                return Ok(None);
            };
            let supervisor = SupervisorView::probe(&paths);
            // Stillborn: created but never started — pending, a dead/absent
            // supervisor, zero nodes, and no forward progress since creation
            // (issue `supervisor-dies-before-worker-node`). Kind-agnostic and
            // derived purely from the manifest + the pid probe we already hold,
            // so it costs no extra I/O and is one consistent shared-lock
            // snapshot with `status`. Mirrors the same detector `run show` /
            // `run wait` already use, so a stillborn run is no longer a silent
            // `pending` row here that looks stuck until someone notices.
            //
            // Age-gated (unlike `show`/`wait`): a bulk `run list` sweeps runs
            // that another process is mid-`run create` on, which transiently
            // present the same shape — see [`STILLBORN_LIST_GRACE_SECS`]. Only a
            // run older than the max create window is flagged. The probe is a
            // racy liveness observation the shared lock cannot freeze, so a
            // recovery action must re-verify under an exclusive lock; this flag
            // is advisory.
            let stillborn = crate::run::stalled::is_stillborn(
                m.status,
                // Only a *confirmed* not-running supervisor flags a run stillborn;
                // an `Unreadable`/`Unknown` (indeterminate) state must not, or we
                // recreate the conflation this DTO change fixes.
                supervisor.presumed_working(),
                m.node_count,
                m.created_at,
                m.updated_at,
            ) && now.signed_duration_since(m.created_at) > stillborn_grace;
            Ok(Some((m, supervisor, stillborn)))
        })
        .map_err(from_core)?;
        let (m, supervisor, stillborn) = match scanned {
            Some(v) => v,
            None => continue, // half-initialized run dir; skip silently
        };
        // `stalled` is the "pending but visibly not progressing" hint; since the
        // 0.2 cut removed the orchestrate driver (the only other stall shape),
        // it is now exactly the never-started `stillborn` variant.
        let stalled = stillborn;
        // Shape the manifest into its wire DTO once, then filter on the
        // canonical kebab strings it carries — the DTO's `From` renders
        // `kind` / `status` through the `run/mod.rs` helpers rather than
        // round-tripping the enums through `serde_json::to_value`.
        let summary = RunSummary::from(&m)
            .with_supervisor(supervisor)
            .with_stalled(stalled)
            .with_stillborn(stillborn);
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
                let sup = match r.supervisor.state {
                    SupervisorState::Alive => match r.supervisor.pid {
                        Some(pid) => format!("sup:alive({pid})"),
                        None => "sup:alive".to_string(),
                    },
                    SupervisorState::Dead => match r.supervisor.pid {
                        Some(pid) => format!("sup:dead({pid})"),
                        None => "sup:dead".to_string(),
                    },
                    SupervisorState::NotRecorded => "sup:none".to_string(),
                    SupervisorState::Unreadable => "sup:unreadable".to_string(),
                    SupervisorState::Unknown => "sup:unknown".to_string(),
                };
                // The status column carries a marker so a plain-text `run list`
                // no longer shows a zombie as an ordinary live `pending` run:
                // `(stillborn)` for a never-started run (supervisor died before
                // creating any worker node), `(stalled)` for an undriven
                // orchestrate driver. The two are mutually exclusive by
                // construction; stillborn is the more specific, so it wins.
                let status = if r.stillborn {
                    format!("{} (stillborn)", r.status)
                } else if r.stalled {
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
