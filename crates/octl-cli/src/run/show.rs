//! `run show` — full manifest + counters for one run.

use serde::Serialize;
use serde_json::Value;

use octl_core::{read_manifest_opt, read_node_opt, NodeId, RunLock, Status};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::dto::{ManifestView, SupervisorView};
use crate::run::{from_core, run_paths};

/// The single reporting node of a single-worker worktree run (`n-0001`);
/// mirrors `run merge` / `run wait`'s `DEFAULT_NODE_ID`. Its terminal report is
/// where a supervisor stamps the `recoverable_work` stranded-work signal.
const DEFAULT_NODE_ID: &str = "n-0001";

#[derive(Serialize)]
struct ShowPayload<'a> {
    manifest: ManifestView<'a>,
    counts: Counts,
    /// Rebase-robust landing signal for the reporting node: true when the
    /// worker's committed work has landed in the target, confirmed by patch-id
    /// equivalence against the *current* target tip (`git cherry`) — NOT by
    /// branch-ref ancestry, which a caller-side `git rebase` invalidates. Falls
    /// back to the durable merge marker when git verification is unavailable
    /// (issue `landing-signal-reliable-after-rebase`). Callers should trust this
    /// instead of running `git merge-base --is-ancestor` by hand.
    landed: bool,
    /// How `landed` was decided: `git-verified` | `report-marker` | `unverified`.
    landed_method: &'static str,
    /// The `recoverable_work` block from the default node's terminal report,
    /// present only when a dead agent left unmerged commits ahead of source
    /// (issue `agent-death-strands-recoverable-work`). Surfaced so a caller can
    /// spot salvageable work on a `failed` run without inspecting the node
    /// projection or running `git log <source>..<branch>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    recoverable_work: Option<Value>,
    /// Computed hint (never persisted): true for an undriven `--kind
    /// orchestrate` driver run whose driver node has sat `pending` with zero
    /// children and no fresh events past the grace window — the silent-zombie
    /// signature from issue `peculiarly-muddled-caption`. `false` for every
    /// other kind, and for an orchestrate run that has spawned children / is
    /// emitting events / is still inside the grace window. See
    /// [`crate::run::stalled`].
    stalled: bool,
}

/// The run/node fields read under the shared lock to compute `landed` after the
/// lock is released (the git shell-out must not run while the flock is held).
struct LandingFields {
    source_repo: Option<String>,
    source_branch: Option<String>,
    worktree_path: Option<String>,
    branch: Option<String>,
    base_sha: Option<String>,
    report: Option<Value>,
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
        // Read the reporting node ONCE inside the shared-lock window, alongside
        // the manifest/counters, so the `landed` git-verification inputs and the
        // `recoverable_work` signal are a single consistent snapshot with
        // `manifest.status` (state-integrity invariant 3). The git shell-out that
        // turns these inputs into `landed` runs AFTER the lock is released — never
        // hold the flock across a subprocess.
        let node_id =
            NodeId::parse_str(DEFAULT_NODE_ID).expect("DEFAULT_NODE_ID is a valid node id");
        let node = read_node_opt(&paths, &node_id)?;
        // `recoverable_work` is gated on a `failed` status: the supervisor only
        // stamps the block on the failed-synthesis path, so a block on a
        // non-failed report is stale/spoofed (unknown report fields are permitted
        // by the validator) and is not surfaced. A run with no `n-0001` node (or
        // no such block) simply yields `None`.
        let recoverable_work = if matches!(manifest.status, Status::Failed) {
            node.as_ref()
                .and_then(|n| n.last_report.as_ref())
                .and_then(|r| r.get("recoverable_work").cloned())
                .filter(Value::is_object)
        } else {
            None
        };
        // Computed stall hint (issue `peculiarly-muddled-caption`): derived
        // purely from the driver node's status/children/`updated_at`, which we
        // already hold under the shared lock — no event/reducer/schema path is
        // touched. Decided here, inside the lock, so it is one consistent
        // snapshot with `manifest.status`.
        let stalled =
            crate::run::stalled::is_stalled(manifest.kind, node.as_ref(), chrono::Utc::now());
        let landing = LandingFields {
            source_repo: manifest.source_repo.clone(),
            source_branch: manifest.source_branch.clone(),
            worktree_path: node.as_ref().and_then(|n| n.worktree_path.clone()),
            branch: node.as_ref().and_then(|n| n.branch.clone()),
            base_sha: node.as_ref().and_then(|n| n.base_sha.clone()),
            report: node.and_then(|n| n.last_report),
        };
        Ok(Some((
            manifest,
            counts,
            supervisor,
            recoverable_work,
            stalled,
            landing,
        )))
    })
    .map_err(from_core)?;
    let (manifest, counts, supervisor, recoverable_work, stalled, landing) = match scanned {
        Some(v) => v,
        None => {
            return Err(
                CliError::user("run_not_found", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
    };
    // Git-verified `landed` (issue `landing-signal-reliable-after-rebase`),
    // computed outside the shared lock: the rebase-robust signal a caller should
    // trust instead of hand-rolling `git merge-base --is-ancestor`.
    let signal = crate::run::landed::landing_signal(
        &crate::run::landed::LandingInputs {
            source_repo: landing.source_repo.as_deref(),
            source_branch: landing.source_branch.as_deref(),
            worktree_path: landing.worktree_path.as_deref(),
            branch: landing.branch.as_deref(),
            base_sha: landing.base_sha.as_deref(),
            report: landing.report.as_ref(),
        },
        &crate::supervise::cleanup::git_bin(),
    );
    let payload = ShowPayload {
        manifest: ManifestView::from(&manifest).with_supervisor(supervisor),
        counts,
        landed: signal.landed,
        landed_method: signal.method.wire(),
        recoverable_work,
        stalled,
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
            if payload.stalled {
                println!(
                    "stalled:       true (undriven orchestrate driver — no children, idle > {} min; \
                     `run cancel {}` and relaunch with an active orchestrator)",
                    crate::run::stalled::STALL_GRACE.num_minutes(),
                    payload.manifest.run_id
                );
            }
            println!(
                "landed:        {} ({})",
                payload.landed, payload.landed_method
            );
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
            if let Some(line) =
                crate::run::wait::recoverable_summary(payload.recoverable_work.as_ref())
            {
                println!("recoverable:   {line}");
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
