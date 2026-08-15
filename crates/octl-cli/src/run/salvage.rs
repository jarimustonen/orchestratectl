//! `run salvage` — the fenced manual resume/finish operation (design.md §2.2 /
//! A3, issue `run-salvage-command`).
//!
//! The thin supervisor deliberately drops the *automatic* rescue of a run that
//! finished-but-skipped-`run merge`, or whose worker wedged. Its replacement is
//! this human-invoked verb: the PO sees an `attention-required` run (design.md
//! §2.5, surfaced by `run list` / `run show` / `run wait`) or a `failed` run
//! whose branch the teardown gate preserved (invariant 5), and drives it to a
//! clean finish WITHOUT talking to the (possibly wedged, possibly already-dead)
//! agent and WITHOUT spawning a second writer into the same worktree.
//!
//! ## What it does (the fenced finish)
//!
//! 1. **Snapshot under the run lock** (invariant #3): read `manifest` + the
//!    single worker node `n-0001` in one shared-locked window, so the refusal
//!    decision never sees a half-applied projection set.
//! 2. **Refuse the cases it must not touch** (see [`run`]): an already-`done` or
//!    `cancelled` run (nothing to salvage / a deliberate teardown), a multi-node
//!    run (ambiguous — fan-out per-node salvage is a follow-up), a run with no
//!    preserved worktree/branch, and a live worker it cannot *safely* fence.
//! 3. **Verify the prior worker's identity, then fence it.** The worker is
//!    classified from durable told facts ([`WorkerState`]): an already-exited
//!    worker (the attention-required happy path) needs no fence; a confirmed-dead
//!    or recycled pid is gone; a *live* worker is fenced with `SIGTERM` — but
//!    ONLY when its recorded start-time identity positively matches, so salvage
//!    can never signal an unrelated process that recycled the pid. Fencing a live
//!    worker requires the explicit `--fence` opt-in (killing a process is
//!    destructive); without it a live worker is a refusal.
//! 4. **Drive `run merge` from the worktree's current git state.** Salvage does
//!    NOT hand-roll a raw git self-merge — that would bypass the
//!    merge-transaction record (invariant 6), the CAS-guarded source fast-forward,
//!    the `via: "explicit-merge"` terminal report, and the supervisor teardown
//!    gate. It delegates to the exact same machinery as `run merge`
//!    ([`crate::run::merge::execute`]), so the terminal report/provenance and
//!    every state-integrity invariant hold identically. Idempotent against a
//!    duplicate merge (the merge path's crash-recovery + idempotency key), so a
//!    re-run after a partial salvage completes cleanly.
//!
//! ## Scope (0.2)
//!
//! This ships the direct finish/merge path plus the explicit refusal modes. The
//! *fresh-agent continuation* variant (design.md §2.2 option (b) — launch one new
//! agent to continue the work instead of merging as-is) is deferred to the
//! follow-up `run-salvage-fresh` so the 0.2 mechanism stays small and
//! safe.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use octl_core::{read_manifest_opt, read_node_opt, Node, NodeId, RunLock, Status};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::merge::{self, ConsumerOutcome};
use crate::run::{from_core, run_paths_from_cli_arg};
use crate::supervise::{pid_file, watchdog};

/// The single reporting node every salvageable (single-worker) run carries.
const DEFAULT_NODE_ID: &str = "n-0001";

/// How long to wait for a fenced worker to actually exit after `SIGTERM` before
/// giving up. A cooperating agent dies well within this; a process that ignores
/// `SIGTERM` past it is reported as an un-fenceable refusal rather than merged
/// out from under a still-live writer.
const FENCE_GRACE: Duration = Duration::from_secs(5);

/// Poll cadence while waiting for a fenced worker to exit.
const FENCE_POLL: Duration = Duration::from_millis(100);

pub struct Args<'a> {
    pub run_id: String,
    /// Override the merge target branch (forwarded to `run merge`). Defaults to
    /// the run's recorded `source_branch`.
    pub source: Option<String>,
    /// Optional §7.3 report payload (JSON file) to submit on the salvage merge,
    /// forwarded verbatim to `run merge` (stamped `via: "explicit-merge"`).
    pub report_file: Option<std::path::PathBuf>,
    /// Permit fencing a *live* worker: `SIGTERM` the recorded agent pid (only
    /// ever when its start-time identity matches). Without it, a live worker is a
    /// refusal — salvage never kills a process implicitly.
    pub fence: bool,
    /// Resolve inputs and report the planned salvage (worker state, whether a
    /// fence would fire, the planned merge) without fencing or merging anything.
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

/// The classified state of the run's prior worker, from durable told facts. The
/// fence decision is a total function of this (see [`run`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerState {
    /// The launcher shim recorded a `worker.exited` — the process is gone (this
    /// is the attention-required / told-failure shape). No fence needed.
    Exited,
    /// No `agent_pid` was ever recorded — nothing to fence.
    NoPid,
    /// The recorded pid is dead, or alive-but-recycled (its start-time no longer
    /// matches) — the original worker is gone. No fence needed, no risk.
    Gone,
    /// The recorded pid is alive AND its start-time identity positively matches —
    /// the original worker is genuinely still running. Safe to `SIGTERM` (behind
    /// `--fence`).
    Live { pid: u32 },
    /// The recorded pid is alive but its identity cannot be confirmed (no recorded
    /// start-time, or the platform declined to read it) — it *might* be a recycled
    /// pid now owned by an unrelated process. Never fenced: a refusal.
    Unverifiable { pid: u32 },
}

impl WorkerState {
    /// The stable machine string surfaced in the payload / refusals.
    fn wire(self) -> &'static str {
        match self {
            WorkerState::Exited => "exited",
            WorkerState::NoPid => "no-pid",
            WorkerState::Gone => "gone",
            WorkerState::Live { .. } => "live",
            WorkerState::Unverifiable { .. } => "unverifiable",
        }
    }
}

/// Classify the worker purely from the node's durable facts. Told beats guessed:
/// a recorded `worker.exited` short-circuits the pid probe entirely (the process
/// already exited — it can't come back). Only when nothing was told does pid
/// liveness + the §7.6 start-time identity defense govern, and identity is
/// *required* to reach [`WorkerState::Live`] so a fence can never signal a
/// recycled pid owned by someone else.
fn classify_worker(node: &Node) -> WorkerState {
    if node.worker_exit.is_some() {
        return WorkerState::Exited;
    }
    let Some(pid_i) = node.agent_pid else {
        return WorkerState::NoPid;
    };
    if pid_i <= 0 {
        return WorkerState::NoPid;
    }
    let pid = pid_i as u32;
    if !pid_file::pid_alive(pid) {
        return WorkerState::Gone;
    }
    // Alive. Confirm it is still the ORIGINAL worker via the recorded start-time
    // (mirrors the watchdog's recycle check: seconds, 1s tolerance).
    let recorded = node
        .agent_pid_start_time
        .map(|t| t.timestamp().max(0) as u64);
    match recorded {
        Some(expected) => match watchdog::pid_start_time(pid) {
            // Start-time matches → genuinely the original, live worker.
            Some(actual) if expected.abs_diff(actual) <= 1 => WorkerState::Live { pid },
            // Start-time disagrees → the pid was recycled; the original is gone.
            Some(_) => WorkerState::Gone,
            // Alive but the platform won't read the start-time → cannot confirm.
            None => WorkerState::Unverifiable { pid },
        },
        // No recorded identity → cannot prove this pid is our worker. Refuse to
        // fence it (it may be a recycled pid now owned by an unrelated process).
        None => WorkerState::Unverifiable { pid },
    }
}

/// `SIGTERM` a verified-live worker and wait (bounded) for it to exit. Returns
/// `Ok(())` once the process is gone, or a `fence_failed` error if it survives
/// the grace (so salvage refuses rather than merge out from under a live writer).
fn fence_worker(pid: u32) -> Result<(), CliError> {
    let Some(pid_t) = pid_file::to_pid_t(pid) else {
        // Unreachable: `classify_worker` already range-checked the pid via
        // `pid_alive`, but stay defensive rather than cast a bad pid into kill().
        return Err(CliError::system(
            "fence_failed",
            format!("worker pid {pid} is out of range; refusing to signal"),
        ));
    };
    // SAFETY: `pid_t` is range-checked (never 0/negative → no group/broadcast
    // target), and SIGTERM is a routine cooperative-termination signal.
    let rc = unsafe { libc::kill(pid_t, libc::SIGTERM) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        // ESRCH: the worker exited between our identity check and the signal — a
        // benign race, the fence is already effectively done.
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(CliError::system(
            "fence_failed",
            format!("SIGTERM to worker pid {pid} failed: {err}"),
        ));
    }
    // Wait for the process to actually exit so the merge below never races a
    // still-live writer.
    let deadline = Instant::now() + FENCE_GRACE;
    while Instant::now() < deadline {
        if !pid_file::pid_alive(pid) {
            return Ok(());
        }
        std::thread::sleep(FENCE_POLL);
    }
    if pid_file::pid_alive(pid) {
        return Err(CliError::user(
            "fence_failed",
            format!(
                "worker pid {pid} did not exit within {}s of SIGTERM — refusing to \
                 merge over a live worker. Investigate the process (it may be blocked \
                 in an uninterruptible state) before retrying.",
                FENCE_GRACE.as_secs()
            ),
        ));
    }
    Ok(())
}

/// The salvage-merge sub-result surfaced in the payload — the machine-readable
/// half of the delegated `run merge`.
#[derive(Serialize)]
struct MergeSummary {
    branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    merged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    report_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supervisor: Option<ConsumerOutcome>,
}

#[derive(Serialize)]
struct SalvagePayload {
    run_id: String,
    node_id: String,
    /// The classified prior-worker state (`exited` | `no-pid` | `gone` | `live` |
    /// `unverifiable`) at salvage time.
    worker_state: &'static str,
    /// Whether salvage fenced (SIGTERM'd) a live worker. `false` when the worker
    /// was already gone; under `--dry-run` this is what a real run *would* do.
    fenced: bool,
    /// The delegated `run merge` result.
    merge: MergeSummary,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    dry_run: bool,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, &args.run_id)?;
    // `args.run_id` may have been an unambiguous prefix; report the resolved id.
    let run_id = paths.run_id.as_str().to_string();
    let node_id = NodeId::parse_str(DEFAULT_NODE_ID).expect("DEFAULT_NODE_ID is valid");

    // One shared-locked read of manifest + node (invariant #3): the refusal
    // decision reasons across both projections, so it must not observe a
    // half-applied set.
    let (manifest, node) = RunLock::with_shared_lock(&paths.lock(), || {
        let manifest = read_manifest_opt(&paths)?;
        let node = read_node_opt(&paths, &node_id)?;
        Ok((manifest, node))
    })
    .map_err(from_core)?;

    let manifest = manifest.ok_or_else(|| {
        CliError::user("run_not_found", format!("no run with id {run_id}"))
            .with_invalid_value(&run_id)
    })?;

    // A run recorded under a removed kind is read-only (ADR §D7) — refuse before
    // any fence/merge so we never rewrite its manifest / destroy its provenance.
    crate::run::reject_legacy_kind(manifest.kind, &run_id)?;

    // Refuse the terminal states there is nothing to salvage from.
    match manifest.status {
        // Already succeeded — the work is merged and the worktree torn down.
        Status::Done => {
            return Err(CliError::user(
                "run_already_terminal",
                format!(
                    "run {run_id} is already done — its work merged and its worktree was \
                     torn down; there is nothing to salvage"
                ),
            )
            .with_invalid_value(&run_id));
        }
        // A deliberate `run cancel` teardown. The reducer never adopts a merge
        // against a cancelled node (see run merge), so a salvage merge would do
        // nothing but strand state. Cancel is final.
        Status::Cancelled => {
            return Err(CliError::user(
                "run_already_terminal",
                format!(
                    "run {run_id} was cancelled — a cancelled run never adopts a merge, so \
                     salvage cannot finish it"
                ),
            )
            .with_invalid_value(&run_id));
        }
        // Pending / Running (incl. attention-required) / Blocked / Failed with a
        // preserved worktree: salvageable.
        Status::Pending | Status::Running | Status::Blocked | Status::Failed => {}
    }

    // Single-worker only. A fan-out / multi-node run is ambiguous — which node's
    // worktree to fence and finish? Per-node salvage is the delegated follow-up
    // (`per-node-run`); refuse here rather than silently pick n-0001.
    if manifest.node_count != 1 {
        return Err(CliError::user(
            "ambiguous_multi_node",
            format!(
                "run {run_id} has {} nodes — salvage targets a single-worker run only. \
                 Per-node salvage of a fan-out is not yet supported.",
                manifest.node_count
            ),
        )
        .with_invalid_value(&run_id)
        .with_expected(serde_json::json!({ "node_count": 1 })));
    }

    let node = node.ok_or_else(|| {
        CliError::user(
            "node_not_found",
            format!("run {run_id} has no {node_id} node to salvage"),
        )
        .with_invalid_value(node_id.as_str())
    })?;

    // The preserved worktree + branch are what salvage finishes. Their absence is
    // a distinct, actionable refusal (a driver node, or work already torn down).
    let worktree_path = node.worktree_path.as_deref().ok_or_else(|| {
        CliError::user(
            "no_worktree",
            format!(
                "node {node_id} has no preserved worktree — nothing to salvage (a driver \
                 node, or the worktree was already removed)"
            ),
        )
        .with_invalid_value(node_id.as_str())
    })?;
    let has_branch = node.branch.as_deref().is_some_and(|s| !s.is_empty());
    if !has_branch {
        return Err(CliError::user(
            "no_branch",
            format!("node {node_id} has no preserved branch recorded; cannot salvage"),
        )
        .with_invalid_value(node_id.as_str()));
    }
    // The worktree must still be on disk for the merge to `cd` into it. A
    // definitely-absent path is a refusal; an "unknown" (permission) stat falls
    // through so the merge surfaces the true error.
    if !Path::new(worktree_path).try_exists().unwrap_or(true) {
        return Err(CliError::user(
            "worktree_missing",
            format!(
                "worktree {worktree_path} no longer exists — its work was likely already \
                 merged or torn down; there is nothing to salvage"
            ),
        )
        .with_invalid_value(&run_id));
    }

    // Classify + decide the fence. This is the whole safety gate.
    let worker = classify_worker(&node);
    let needs_fence = match worker {
        WorkerState::Exited | WorkerState::NoPid | WorkerState::Gone => false,
        WorkerState::Unverifiable { pid } => {
            // Alive but unidentifiable — could be a recycled pid owned by an
            // unrelated process. NEVER fence it, even with --fence.
            return Err(CliError::user(
                "worker_unfenceable",
                format!(
                    "run {run_id}'s worker pid {pid} is alive but its identity cannot be \
                     verified (no recorded start-time) — refusing to signal a process that \
                     may have been recycled. Confirm the worker is gone, then retry."
                ),
            )
            .with_invalid_value(&run_id));
        }
        WorkerState::Live { pid } => {
            if !args.fence {
                return Err(CliError::user(
                    "worker_live",
                    format!(
                        "run {run_id}'s original worker (pid {pid}) is still alive. Salvage \
                         will not kill a running worker implicitly — re-run with `--fence` to \
                         SIGTERM it and finish the run, or let it complete on its own."
                    ),
                )
                .with_invalid_value(&run_id));
            }
            true
        }
    };

    // Dry run: report the plan (worker state, whether a fence would fire, the
    // planned merge) without fencing or merging.
    if args.dry_run {
        let mo = merge::execute(&merge::Args {
            run_id: run_id.clone(),
            source: args.source.clone(),
            node_id: None,
            report_file: args.report_file.clone(),
            dry_run: true,
            spec: args.spec,
            warnings: args.warnings,
        })?;
        let warnings = mo.warnings.clone();
        return emit(
            &SalvagePayload {
                run_id,
                node_id: node_id.as_str().to_string(),
                worker_state: worker.wire(),
                fenced: needs_fence,
                merge: merge_summary(&mo),
                dry_run: true,
            },
            args.spec,
            &warnings,
        );
    }

    // Fence the (verified-live) worker before merging so no second writer races
    // the worktree's git state.
    let mut base_warnings: Vec<String> = args.warnings.to_vec();
    let fenced = if needs_fence {
        if let WorkerState::Live { pid } = worker {
            fence_worker(pid)?;
            base_warnings.push(format!(
                "fenced worker pid {pid} (SIGTERM) before finishing the run"
            ));
        }
        true
    } else {
        false
    };

    // Drive the merge through the exact `run merge` machinery — crash-recovery,
    // CAS-guarded source FF, `via: "explicit-merge"` terminal report, supervisor
    // reattach — so provenance and every state-integrity invariant hold. Our
    // fence note is seeded as the base warnings, so the merge's
    // `ensure_report_consumer` appends to the same list the envelope emits.
    let mo = merge::execute(&merge::Args {
        run_id: run_id.clone(),
        source: args.source.clone(),
        node_id: None,
        report_file: args.report_file.clone(),
        dry_run: false,
        spec: args.spec,
        warnings: &base_warnings,
    })?;
    let out_warnings = mo.warnings.clone();

    emit(
        &SalvagePayload {
            run_id,
            node_id: node_id.as_str().to_string(),
            worker_state: worker.wire(),
            fenced,
            merge: merge_summary(&mo),
            dry_run: false,
        },
        args.spec,
        &out_warnings,
    )
}

fn merge_summary(mo: &merge::MergeOutcome) -> MergeSummary {
    MergeSummary {
        branch: mo.branch.clone(),
        source: mo.source.clone(),
        merged: mo.merged,
        report_seq: mo.report_seq,
        supervisor: mo.supervisor.clone(),
    }
}

fn emit(payload: &SalvagePayload, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:       {}", payload.run_id);
            println!("node-id:      {}", payload.node_id);
            println!("worker-state: {}", payload.worker_state);
            println!("fenced:       {}", payload.fenced);
            println!("branch:       {}", payload.merge.branch);
            match &payload.merge.source {
                Some(s) => println!("source:       {s}"),
                None => println!("source:       (auto-detect main/master)"),
            }
            if payload.dry_run {
                println!("note:         --dry-run (no fence, no merge)");
            } else {
                println!("merged:       {}", payload.merge.merged);
                if let Some(seq) = payload.merge.report_seq {
                    println!("report_seq:   {seq}");
                }
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    use chrono::{DateTime, Utc};
    use octl_core::{Kind, RunId, WorkerExit};

    /// A minimal `n-0001` node with the pid/exit fields under test set.
    fn node(
        agent_pid: Option<i32>,
        start_time: Option<DateTime<Utc>>,
        exit: Option<WorkerExit>,
    ) -> Node {
        Node {
            schema_version: 1,
            node_id: NodeId::parse_str("n-0001").unwrap(),
            run_id: RunId::parse_str("01jxsnap000000000000000000").unwrap(),
            parent_node_id: None,
            kind: Kind::Spinoff,
            status: Status::Running,
            task: None,
            worktree_path: Some("/tmp/wt".into()),
            branch: Some("wt/x".into()),
            base_sha: None,
            tmux_window: None,
            tmux_identity: None,
            agent_pid,
            agent_pid_start_time: start_time,
            supervisor_pid: None,
            children: vec![],
            started_at: None,
            updated_at: Utc::now(),
            last_report: None,
            last_processed_report_seq_by_child: serde_json::Map::new(),
            retry_attempts: 0,
            worker_exit: exit,
            pending_merge: None,
            first_death_at: None,
        }
    }

    fn spawn_sleeper() -> std::process::Child {
        Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep")
    }

    /// A recorded `worker.exited` short-circuits everything: the process is gone,
    /// regardless of any pid (told beats guessed).
    #[test]
    fn told_exit_is_exited_even_with_live_pid() {
        let mut child = spawn_sleeper();
        let exit = WorkerExit {
            code: Some(0),
            signal: None,
            at: Utc::now(),
        };
        let n = node(Some(child.id() as i32), None, Some(exit));
        assert_eq!(classify_worker(&n), WorkerState::Exited);
        let _ = child.kill();
        let _ = child.wait();
    }

    /// No recorded pid → nothing to fence.
    #[test]
    fn no_pid_is_no_pid() {
        assert_eq!(classify_worker(&node(None, None, None)), WorkerState::NoPid);
        // A non-positive pid is treated as "no pid" (never a signal target).
        assert_eq!(
            classify_worker(&node(Some(0), None, None)),
            WorkerState::NoPid
        );
    }

    /// A dead recorded pid is `Gone` — safe to salvage, nothing to fence.
    #[test]
    fn dead_pid_is_gone() {
        let mut child = spawn_sleeper();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        // The pid is now dead (reaped). A recorded start-time is irrelevant.
        assert_eq!(
            classify_worker(&node(Some(pid as i32), None, None)),
            WorkerState::Gone
        );
    }

    /// A live pid whose recorded start-time matches is the original, live worker —
    /// `Live` (fenceable). A live pid with NO recorded start-time is
    /// `Unverifiable` (never fenced).
    #[test]
    fn live_pid_identity_gates_fenceability() {
        let mut child = spawn_sleeper();
        let pid = child.id();
        let st = watchdog::pid_start_time(pid).expect("read child start_time");
        let recorded = DateTime::from_timestamp(st as i64, 0).unwrap();

        assert_eq!(
            classify_worker(&node(Some(pid as i32), Some(recorded), None)),
            WorkerState::Live { pid }
        );
        assert_eq!(
            classify_worker(&node(Some(pid as i32), None, None)),
            WorkerState::Unverifiable { pid }
        );
        // A recorded start-time that disagrees (1970) → the pid was recycled → Gone.
        let bogus = DateTime::from_timestamp(1, 0).unwrap();
        assert_eq!(
            classify_worker(&node(Some(pid as i32), Some(bogus), None)),
            WorkerState::Gone
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    /// `fence_worker` SIGTERMs a live process and returns once it is gone.
    ///
    /// A concurrent reaper thread `wait()`s the child so it does not linger as a
    /// zombie (a zombie still answers `kill(pid, 0)` as alive). In production the
    /// fenced worker is never salvage's own child, so it is reaped by its real
    /// parent and this concern does not arise.
    #[test]
    fn fence_worker_terminates_a_live_process() {
        let mut child = spawn_sleeper();
        let pid = child.id();
        assert!(pid_file::pid_alive(pid));
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });
        fence_worker(pid).expect("fence succeeds");
        reaper.join().unwrap();
        assert!(!pid_file::pid_alive(pid), "worker must be dead after fence");
    }
}
