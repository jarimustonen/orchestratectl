//! `orchestratectl supervise <run-id>` — long-lived per-run supervisor.
//!
//! Owns three cooperating loops (single-threaded polling):
//!   1. **Own-run tail** — react to `child.spawned` (fork a child
//!      supervisor) and `run.status` (terminal → clean exit).
//!   2. **Child-run tails** — react to `node.report` (deterministic-ID
//!      dedup via [`reducer::process_node_report`]) and child `run.status`.
//!   3. **Watchdog** — dual-poll PID + start-time + tmux liveness for
//!      tracked agents, synthesizing terminal `node.report` events when
//!      the agent dies before reporting.
//!
//! Lifecycle: trap SIGINT/SIGTERM via `sigaction` (exit 130 / 143 per
//! §7.8), refuse to launch if the `<run-dir>/supervisor.pid` PID is alive
//! (start-time identity check, §7.6), atomically write our own PID on
//! boot, emit `supervisor.exited` and remove the PID file on exit.
//! `--once` and `--max-iter <n>` are test-only escape hatches.
//!
//! Orphan defense: if our run's `manifest.json` disappears for a few
//! consecutive ticks (the run dir was removed — e.g. a test `TempDir`
//! teardown, or an operator deleting the run), there is nothing left to
//! supervise. We self-terminate cleanly (exit 0, `supervisor.self-terminated`
//! event when the events log survives) rather than poll a deleted
//! directory forever and keep forking children.

pub mod capture;
pub mod cleanup;
pub mod notify;
pub mod pid_file;
pub mod reducer;
pub mod state;
pub mod tail;
pub mod watchdog;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use clap::Args as ClapArgs;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use octl_core::{
    append_and_apply_event, append_and_apply_unlocked, read_manifest_opt, read_node_opt, Lifecycle,
    Node, NodeId, RunLock, RunPaths, Status,
};

use crate::error::{CliError, ExitKind};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_run_id, run_paths, supervisor_readiness};

/// Polling cadences (design.md §7.5 defaults).
const TAIL_TICK: Duration = Duration::from_millis(500);
const WATCHDOG_TICK: Duration = Duration::from_secs(1);
/// Max time we wait for a spawned child run's directory to appear
/// (handoff D1).
const CHILD_DIR_WAIT: Duration = Duration::from_secs(5);

/// How long a freshly-forked child supervisor may sit in `Starting` — forked,
/// but its identity-verified `supervisor.pid` not yet readable — before the
/// parent declares that boot attempt failed and schedules a retry. The child
/// writes its pid file within milliseconds of `exec` under normal load; this
/// bound is deliberately generous so a slow-but-healthy boot is never clipped
/// (issue `child-supervisor-spawn-unconfirmed-no-retry`). Overridable via
/// [`CHILD_SPAWN_DEADLINE_ENV`] so a test can drive the failure path fast.
const CHILD_SPAWN_DEADLINE: Duration = Duration::from_secs(10);

/// Env override for [`CHILD_SPAWN_DEADLINE`] (whole seconds; unparseable →
/// default). Tests set a small value to reach the `Failed`/retry transition
/// without a real 10s wait.
const CHILD_SPAWN_DEADLINE_ENV: &str = "OCTL_CHILD_SPAWN_DEADLINE_SECS";

/// Base backoff before re-forking a child supervisor whose previous attempt
/// never confirmed a pid. Doubles per attempt, capped at
/// [`CHILD_RETRY_MAX_BACKOFF`], so a transient boot failure is retried promptly
/// while a persistently broken environment is not hammered.
const CHILD_RETRY_BASE_BACKOFF: Duration = Duration::from_secs(2);

/// Ceiling for the [`CHILD_RETRY_BASE_BACKOFF`] exponential backoff.
const CHILD_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Max child-supervisor boot attempts before the parent stops retrying a child
/// that never confirms a pid. Bounded so a permanently broken environment
/// cannot spin the fork path forever; every failed attempt leaves a
/// `child.spawn_failed` event on the parent log for operators.
const CHILD_SPAWN_MAX_ATTEMPTS: u32 = 5;

/// The effective child-spawn deadline, honoring [`CHILD_SPAWN_DEADLINE_ENV`].
fn child_spawn_deadline() -> Duration {
    match std::env::var(CHILD_SPAWN_DEADLINE_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .map_or(CHILD_SPAWN_DEADLINE, Duration::from_secs),
        Err(_) => CHILD_SPAWN_DEADLINE,
    }
}

/// Max bounded auto-retries of an autonomous single-node worker that dies
/// EMPTY-HANDED (`agent-died`, nothing committed) before the run is finally
/// terminalized `failed` (issue `autoretry-agent-died-worker`). Set to 3 from the
/// observed evidence — the `pipeline-tiered-triage` task died at ~13min twice and
/// landed cleanly on the third spawn — so a transient death recovers without a
/// human. Bounded so a deterministically-dying agent can never respawn forever.
/// The count is DURABLE (`Node.retry_attempts`), so the bound holds across
/// supervisor restarts. Overridable via [`AGENT_RETRY_MAX_ATTEMPTS_ENV`].
const AGENT_RETRY_MAX_ATTEMPTS: u32 = 3;

/// Env override for [`AGENT_RETRY_MAX_ATTEMPTS`] (whole count; unparseable →
/// default). Tests set a small value to reach the exhausted-`failed` transition
/// quickly.
const AGENT_RETRY_MAX_ATTEMPTS_ENV: &str = "OCTL_AGENT_RETRY_MAX_ATTEMPTS";

/// Base backoff before re-spawning a dead empty-handed worker. Doubles per
/// attempt, capped at [`AGENT_RETRY_MAX_BACKOFF`], so a fast-recurring transient
/// death is not hammered while a genuinely broken run still terminalizes bounded.
const AGENT_RETRY_BASE_BACKOFF: Duration = Duration::from_secs(10);

/// Ceiling for the [`AGENT_RETRY_BASE_BACKOFF`] exponential backoff.
const AGENT_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(120);

/// Env override for [`AGENT_RETRY_BASE_BACKOFF`] (whole seconds; unparseable →
/// default). Tests set `0` so the reconcile re-spawns on the next tick without a
/// real backoff wait.
const AGENT_RETRY_BACKOFF_ENV: &str = "OCTL_AGENT_RETRY_BACKOFF_SECS";

/// Max consecutive `create.sh` failures while re-spawning ONE parked node before
/// the run is terminalized `failed`. Distinct from a dying agent (a broken spawn
/// infrastructure, e.g. a missing `create.sh` or exhausted PTYs): bounded in
/// memory so the reconcile can never loop forever on a host that cannot spawn.
/// Overridable via [`AGENT_RESPAWN_MAX_FAILURES_ENV`].
const AGENT_RESPAWN_MAX_FAILURES: u32 = 3;

/// Env override for [`AGENT_RESPAWN_MAX_FAILURES`] (whole count; unparseable →
/// default). Tests set a small value to reach the spawn-failure terminal path fast.
const AGENT_RESPAWN_MAX_FAILURES_ENV: &str = "OCTL_AGENT_RESPAWN_MAX_FAILURES";

/// The effective spawn-failure budget, honoring [`AGENT_RESPAWN_MAX_FAILURES_ENV`].
fn agent_respawn_max_failures() -> u32 {
    match std::env::var(AGENT_RESPAWN_MAX_FAILURES_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u32>()
            .map_or(AGENT_RESPAWN_MAX_FAILURES, |n| n),
        Err(_) => AGENT_RESPAWN_MAX_FAILURES,
    }
}

/// The effective max retry attempts, honoring [`AGENT_RETRY_MAX_ATTEMPTS_ENV`].
fn agent_retry_max_attempts() -> u32 {
    match std::env::var(AGENT_RETRY_MAX_ATTEMPTS_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u32>()
            .map_or(AGENT_RETRY_MAX_ATTEMPTS, |n| n),
        Err(_) => AGENT_RETRY_MAX_ATTEMPTS,
    }
}

/// Bounded exponential backoff before the re-spawn that follows `attempt` (≥1):
/// `BASE * 2^(attempt-1)`, capped at [`AGENT_RETRY_MAX_BACKOFF`]. The shift is
/// clamped so the doubling can never overflow `Duration`. Honors
/// [`AGENT_RETRY_BACKOFF_ENV`] (`0` → immediate, for tests).
fn agent_retry_backoff(attempt: u32) -> Duration {
    let base_secs = match std::env::var(AGENT_RETRY_BACKOFF_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .unwrap_or_else(|_| AGENT_RETRY_BASE_BACKOFF.as_secs()),
        Err(_) => AGENT_RETRY_BASE_BACKOFF.as_secs(),
    };
    let shift = attempt.saturating_sub(1).min(5);
    // Saturating arithmetic throughout so a large `OCTL_AGENT_RETRY_BACKOFF_SECS`
    // can never overflow the multiply (a panic) or the later `now + backoff`
    // `Instant` add — the `.min(cap)` bounds it to a small ceiling regardless.
    let secs = base_secs
        .saturating_mul(1u64 << shift)
        .min(AGENT_RETRY_MAX_BACKOFF.as_secs());
    Duration::from_secs(secs)
}
/// Consecutive missing-manifest polls (`WATCHDOG_TICK` apart, so ≈3s)
/// before we self-terminate. Defends against orphaning: when a run dir is
/// deleted out from under us (a test's `TempDir` on teardown, or an
/// operator removing the run), there is nothing left to supervise and
/// polling the vanished directory forever wastes CPU + file descriptors.
/// We require a short streak rather than reacting to a single missed read
/// so a transient `stat` hiccup cannot kill a live supervisor.
const SELF_TERMINATE_TICKS: u32 = 3;

/// Consecutive ticks a supervised run may present the "no worker node and no
/// children" state before the no-worker guard considers terminalizing it.
///
/// Every real run has ≥1 node by the time its OWN supervisor boots: a top-level
/// worker's `node.created` (n-0001) is emitted by `run create` *before* it
/// forks the supervisor; a child worker's is emitted before the parent's
/// `child.spawned`; the orchestrate driver synthesizes its `n-0001` node and a
/// fan-out driver materializes one via create.sh. So the ONLY way a supervisor
/// observes zero nodes AND zero tracked/forked children is a `run reattach` (or
/// re-spawn) against a run whose worker was never created — the silent
/// spawn-failure run, or the reattached zombie that would otherwise poll
/// forever / falsely report `work-complete` (issue
/// `supervisor-spawn-fails-silently-at-run-create`, suggested-fix #5).
///
/// The tick streak alone is NOT sufficient, because a run legitimately sits at
/// `node_count == 0` for the whole `create.sh` window (up to the caller's
/// `--agent-startup-timeout`, default 90s, MAX 600s) between `run.created` and
/// `node.created`. A reattach issued during that window would see the same
/// shape. So terminalization is ALSO gated on [`NO_WORKER_GRACE`] — the run
/// must have existed longer than any possible create window — and the predicate
/// is re-verified under the exclusive run lock before the append (below).
const NO_WORKER_TICKS: u32 = 3;

/// Minimum age (from `manifest.created_at`) a zero-node run must reach before
/// the no-worker guard may fail it. Comfortably beyond the maximum create.sh
/// window (`--agent-startup-timeout` caps at 600s) so an in-flight creation is
/// never clipped; the field-reported stuck runs were frozen for >1h, far past
/// this. Overridable via `OCTL_NO_WORKER_GRACE_SECS` (tests set `0`).
const NO_WORKER_GRACE: Duration = Duration::from_secs(900);

/// Env override for [`NO_WORKER_GRACE`] (whole seconds; unparseable → default).
const NO_WORKER_GRACE_ENV: &str = "OCTL_NO_WORKER_GRACE_SECS";

/// The effective no-worker grace, honoring [`NO_WORKER_GRACE_ENV`].
fn no_worker_grace() -> Duration {
    match std::env::var(NO_WORKER_GRACE_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .map_or(NO_WORKER_GRACE, Duration::from_secs),
        Err(_) => NO_WORKER_GRACE,
    }
}

/// Reason recorded on the synthesized terminal `run.status` when the no-worker
/// guard fires. Distinct from a genuine agent failure so an operator can tell a
/// never-created worker from a spawned-then-died one.
const NO_WORKER_REASON: &str = "no-worker-node";

/// How long an alive-but-idle autonomous agent may sit on committed, cleanly-
/// mergeable work WITHOUT calling `run merge` before the idle-unmerged safety net
/// terminalizes the run (issue `agent-skips-run-merge-idle-pending`).
///
/// The observed failure: an autonomous agent finished its work (clean commits,
/// clean worktree) but skipped its mandatory `run merge` close and dropped to an
/// idle shell — so the PID stays Alive, no terminal report ever lands, and the
/// run hangs `pending` forever. This threshold is the bounded quiet-period after
/// which the supervisor stops waiting and terminalizes the run to a recoverable
/// failed state a conductor can salvage.
///
/// Chosen generously (30 min) and measured from the LATER of the branch-tip
/// commit time and the pane-transcript mtime (see
/// [`cleanup::node_idle_unmerged`]), so a legitimately long-running agent — heavy
/// `/llm-review` units run 54–96 min — is never tripped as long as it is still
/// committing OR still emitting pane output. Overridable via
/// [`IDLE_UNMERGED_ENV`]; tests set `0` to fire on the first quiet tick.
const IDLE_UNMERGED_THRESHOLD: Duration = Duration::from_secs(1800);

/// Env override for [`IDLE_UNMERGED_THRESHOLD`] (whole seconds; unparseable →
/// default).
const IDLE_UNMERGED_ENV: &str = "OCTL_IDLE_UNMERGED_SECS";

/// Reason recorded on the synthesized failed `node.report` when the idle-unmerged
/// safety net fires. Distinct from `agent-died` (the process is still Alive) and
/// from a blocked handoff (this is complete-but-unlanded work) so `run show` /
/// `run wait` can tell "agent skipped its close" from a genuine failure or a
/// human-needed tie.
const IDLE_UNMERGED_REASON: &str = "agent-idle-unmerged";

/// The effective idle-unmerged threshold, honoring [`IDLE_UNMERGED_ENV`].
fn idle_unmerged_threshold() -> Duration {
    match std::env::var(IDLE_UNMERGED_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .map_or(IDLE_UNMERGED_THRESHOLD, Duration::from_secs),
        Err(_) => IDLE_UNMERGED_THRESHOLD,
    }
}

/// The idle-unmerged safety net's third activity clock (issue
/// `agent-skips-run-merge-idle-pending`): the Unix time at which `node_id`'s
/// agent process was last seen to have advanced its cumulative CPU time.
///
/// `cpu_now` is the current cumulative CPU time (centiseconds, from
/// [`watchdog::pid_cpu_time_centis`]); `map` remembers `(last observed CPU,
/// Unix time of the last change)` per node across ticks. On the first
/// observation the baseline is seeded to `now_unix` (assume active), so a
/// just-observed node is never immediately judged idle. On later ticks the
/// stored change-time only advances when the CPU total moves — so an agent
/// busy but silent (CPU climbing, no commits, no pane output) keeps a fresh
/// clock and is never tripped, while a genuinely idle agent (CPU flat) lets the
/// clock go stale and the net eventually fire.
///
/// Returns `None` when `cpu_now` is `None` (CPU unreadable) so the caller simply
/// omits this clock and falls back to the commit + pane clocks — never firing on
/// the *absence* of a CPU reading.
fn cpu_activity_clock(
    map: &mut std::collections::BTreeMap<String, (u64, i64)>,
    node_id: &str,
    cpu_now: Option<u64>,
    now_unix: i64,
) -> Option<i64> {
    let cpu = cpu_now?;
    let entry = map.entry(node_id.to_string()).or_insert((cpu, now_unix));
    if entry.0 != cpu {
        *entry = (cpu, now_unix);
    }
    Some(entry.1)
}

/// Max attempts to fire the `--notify` completion hook before the supervisor
/// gives up and winds down anyway. `notify::maybe_fire` returns a retryable
/// failure only when it cannot enter its lock critical section or the marker
/// scan errors (transient I/O / lock contention), so a small bound is plenty;
/// without it a persistent failure (disk full) would spin the loop-exit gate
/// forever. Exhausting the bound is a last-resort miss — logged, and rare —
/// which is acceptable even under the at-least-once policy since the loop must
/// still be able to terminate.
const NOTIFY_MAX_ATTEMPTS: u32 = 5;

/// Minimum age a node must reach before the watchdog will synthesize a
/// terminal `agent-died` (or `agent-tmux-window-gone` / `agent-pid-recycled`)
/// report for it. Within this window the watchdog leaves a non-Alive verdict
/// alone.
///
/// Rationale: `run::spawn::verify_agent_pid` already confirmed the agent PID
/// was alive at the instant `node.created` was emitted, so an apparent
/// "dead"/"recycled"/"tmux-gone" verdict in the first seconds is almost always
/// a *spawn race* (the OS has not finished mapping the PID, `sysinfo` cannot
/// yet read its `start_time`, or the agent has not yet checkpointed that it is
/// alive) rather than a real death. Firing here would terminalize a live,
/// freshly-spawned agent — and, with auto-cleanup landed, destroy its
/// worktree, branch, and tmux window mid-flight. The grace is anchored on the
/// node's `started_at` (the immutable `node.created` timestamp), so it is
/// measured from spawn, not from supervisor start. Overridable via the
/// [`SPAWN_GRACE_ENV`] env var; tests set it to `0` to exercise pure liveness
/// semantics without the delay.
const WATCHDOG_SPAWN_GRACE: Duration = Duration::from_secs(5);

/// Env var that overrides [`WATCHDOG_SPAWN_GRACE`] (whole seconds). `0`
/// disables the grace entirely (a non-Alive verdict fires on the first tick),
/// which is how the liveness-semantics integration tests opt out of the delay.
const SPAWN_GRACE_ENV: &str = "OCTL_WATCHDOG_GRACE_SECS";

/// Minimum gap between successive "log events dropped" warnings. The
/// supervisor never renders a success envelope while it runs, so unlike
/// short-lived commands it cannot surface lossy-mode drops there — it emits a
/// periodic `warn!` instead. Rate-limited so a sustained back-pressure storm
/// does not itself flood the log it is warning about.
const DROPPED_WARN_INTERVAL: Duration = Duration::from_secs(60);

/// Set by the SIGINT/SIGTERM handler to the received signal number (0 =
/// none). Read by the main loop to trigger shutdown and by the shutdown
/// path to pick the §7.8 exit code and `signal` payload field. We use a
/// raw `sigaction` rather than the `ctrlc` crate because §7.8 requires
/// distinguishing SIGINT (exit 130) from SIGTERM (exit 143), and `ctrlc`
/// collapses both into a single edge without surfacing which fired.
static SIGNAL_RECEIVED: AtomicI32 = AtomicI32::new(0);

extern "C" fn handle_term_signal(sig: libc::c_int) {
    // Async-signal-safe: a single compare-exchange. The FIRST signal
    // wins, so a SIGINT racing in during a SIGTERM shutdown cannot flip
    // the recorded signal / exit code out from under the shutdown path.
    let _ = SIGNAL_RECEIVED.compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
}

/// Install SIGINT/SIGTERM handlers via `sigaction`. Fatal on failure: a
/// supervisor that cannot trap signals cannot honor §7.8's clean-shutdown
/// contract (emit `supervisor.exited`, remove its PID file).
fn install_signal_handlers() -> Result<(), CliError> {
    // SAFETY: the handler is async-signal-safe (a single atomic store)
    // and `sa` is zero-initialized then fully populated before use.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_term_signal as extern "C" fn(libc::c_int) as usize;
        // Block both term signals while the handler runs, and use
        // SA_RESTART so a signal arriving mid-syscall (e.g. the `flock`
        // / write inside the shutdown `append_and_apply_event`) does not fail
        // that syscall with EINTR and defeat the clean-shutdown contract.
        libc::sigemptyset(&raw mut sa.sa_mask);
        libc::sigaddset(&raw mut sa.sa_mask, libc::SIGINT);
        libc::sigaddset(&raw mut sa.sa_mask, libc::SIGTERM);
        sa.sa_flags = libc::SA_RESTART;
        for sig in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(sig, &raw const sa, std::ptr::null_mut()) != 0 {
                let err = std::io::Error::last_os_error();
                return Err(CliError::system(
                    "signal_install_failed",
                    format!("sigaction({sig}) failed: {err}"),
                ));
            }
        }
    }
    Ok(())
}

#[derive(ClapArgs, Debug)]
pub struct SuperviseArgs {
    /// Run id to supervise.
    pub run_id: String,
    /// Tick the watchdog + tail loops exactly once, then exit cleanly.
    /// **Test-only escape hatch — never set in production.**
    #[arg(long)]
    pub once: bool,
    /// Tick at most this many iterations, then exit cleanly. **Test-only
    /// escape hatch — never set in production.** Note: `--once` takes
    /// precedence — when both are set the loop still exits after the
    /// first tick, regardless of `--max-iter`.
    #[arg(long)]
    pub max_iter: Option<u32>,
    /// Opt out of corrupt-line quarantine. By default the supervisor heals a
    /// poisoned `events.jsonl` it tails over: the corrupt line is renamed
    /// aside to `events.jsonl.corrupt-<ts>.bak` and a recovered log is written
    /// in its place, then a `supervisor.event_log_quarantined` event is
    /// emitted. With this flag the corrupt line is only skipped in memory and
    /// surfaced via `supervisor.event_log_skipped_line` (the P2 behavior),
    /// leaving the poison bytes on disk for a future strict reader to trip on.
    #[arg(long)]
    pub no_quarantine_corrupt_lines: bool,
}

/// Everything the supervise loop needs, assembled by [`boot_supervisor`]. Boot
/// is fallible and side-effecting (it claims `supervisor.pid` under the run
/// flock and emits `supervisor.started`); extracting it lets [`dispatch`] report
/// boot success/failure down the readiness pipe at a single, clear seam before
/// entering the loop.
struct SupervisorBoot {
    root: PathBuf,
    paths: RunPaths,
    pid_path: PathBuf,
    our_pid: u32,
    state: state::SupervisorState,
    own_tail: tail::EventTail,
    child_tails: std::collections::BTreeMap<String, ChildTracking>,
}

/// Perform the supervisor's boot: resolve paths, verify the run exists, install
/// signal handlers, atomically claim `supervisor.pid`, emit `supervisor.started`,
/// and seed the own-run + child tails. Any error here means the supervisor is
/// NOT going to supervise — [`dispatch`] reports the reason down the readiness
/// pipe and propagates it.
fn boot_supervisor(run_id: &str) -> Result<SupervisorBoot, CliError> {
    let root = crate::home::root_dir()?;
    let paths = run_paths(root.as_path(), run_id)?;
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(CliError {
            kind: ExitKind::User,
            code: "run_not_found".into(),
            message: format!("no run with id {run_id}"),
            invalid_value: Some(run_id.to_string()),
            expected: None,
        });
    }

    let pid_path = paths.supervisor_pid();

    // Reset the process-global signal flag so a prior in-process
    // dispatch (tests, embedded callers) can't poison this run, then
    // install handlers BEFORE claiming the PID file so a signal arriving
    // during startup still drives a clean shutdown, and so a claimed PID
    // file is never left behind by an untrapped signal (§7.8).
    SIGNAL_RECEIVED.store(0, Ordering::SeqCst);
    install_signal_handlers()?;

    let our_pid = std::process::id();
    // Atomically claim ownership under the run flock. This closes the §7.6
    // TOCTOU race where two concurrent `supervise` / reattach-spawned
    // launches both read a stale pid and both write their own: the loser
    // here returns `supervisor_already_running` and exits.
    pid_file::claim_pid_atomic(&paths, our_pid)?;

    // From here the pid file is OURS. If any later boot step fails, remove it
    // before returning so a failed boot does not leave a stale pid record
    // masquerading as a live supervisor (the next `claim` would rely on
    // dead-pid detection to reclaim it). Assemble the rest under a closure so
    // every early `?` funnels through the single cleanup below.
    let assemble = || -> Result<(state::SupervisorState, tail::EventTail, _), CliError> {
        info!(
            target: "orchestratectl::supervise",
            run_id = %run_id,
            pid = our_pid,
            "supervisor started"
        );
        let _ = append_and_apply_event(
            &paths,
            "supervisor.started",
            None,
            None,
            json!({"pid": our_pid}),
        )
        .map_err(from_core);

        let state = state::load(&paths.root)?;
        let own_tail = tail::EventTail::new(paths.events(), state.last_seq_own);
        let mut child_tails: std::collections::BTreeMap<String, ChildTracking> =
            std::collections::BTreeMap::new();
        // Reseed child tails from the canonical node projections, NOT from
        // the private `spawned_children` cache (§7.6: "for each child in the
        // root node's children field, open a tail-follow loop"). The cache
        // can be missing or stale after a crash; the projections are the
        // truth. Each tail resumes from the durable report cursor
        // (`last_processed_report_seq_by_child`) so an un-consumed report is
        // re-tailed rather than skipped.
        for (cid, parent_node_id) in discover_children(&paths) {
            let child_paths = run_paths(&root, &cid)?;
            let seq = state
                .last_processed_report_seq_by_child
                .get(&cid)
                .copied()
                .unwrap_or(0);
            child_tails.insert(
                cid.clone(),
                ChildTracking {
                    parent_node_id,
                    tail: tail::EventTail::new(child_paths.events(), seq),
                    terminal: false,
                },
            );
        }
        Ok((state, own_tail, child_tails))
    };
    let (state, own_tail, child_tails) = match assemble() {
        Ok(v) => v,
        Err(e) => {
            pid_file::remove_if_owner(&pid_path, our_pid);
            return Err(e);
        }
    };

    Ok(SupervisorBoot {
        root,
        paths,
        pid_path,
        our_pid,
        state,
        own_tail,
        child_tails,
    })
}

pub fn dispatch(
    args: SuperviseArgs,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let run_id = args.run_id.clone();

    // Readiness pipe (issue `supervisor-confirm-readiness-pipe`): when `run
    // create` spawned us it passed the write end of a pipe via
    // `OCTL_READINESS_FD`; it is blocked reading it. We confirm boot down that
    // pipe AFTER `claim_pid_atomic` + init succeeds, or report the real reason
    // if boot fails — so the parent never false-fails a slow-but-healthy boot,
    // and never orphans a supervisor it was told died. `from_env` is a no-op
    // when the variable is unset (the lenient spawn paths and direct
    // `supervise` invocations).
    let mut readiness = supervisor_readiness::ReadinessReporter::from_env();

    // Everything that must succeed before the supervise loop is fallible boot.
    // On failure we tell the parent the specific reason (a bare pipe EOF would
    // otherwise read as an unexplained death) and propagate.
    let boot = match boot_supervisor(&run_id) {
        Ok(b) => {
            // A termination signal arriving during boot would set the flag but
            // not stop init; the loop's first tick then exits immediately.
            // Confirming boot in that window would tell the parent "running"
            // about a supervisor already shutting down. Report it as a boot
            // failure and release the pid file instead.
            if SIGNAL_RECEIVED.load(Ordering::SeqCst) != 0 {
                readiness.error(
                    "terminated_during_boot",
                    "termination signal received during supervisor boot",
                );
                pid_file::remove_if_owner(&b.pid_path, b.our_pid);
                return Err(CliError::system(
                    "terminated_during_boot",
                    format!(
                        "supervisor for run {run_id} received a termination signal during boot"
                    ),
                ));
            }
            // Init complete and the pid file is claimed + durable. Confirm boot
            // to the parent BEFORE the (potentially long-blocking) loop.
            readiness.ready(b.our_pid);
            b
        }
        Err(e) => {
            readiness.error(&e.code, &e.message);
            return Err(e);
        }
    };
    let SupervisorBoot {
        root,
        paths,
        pid_path,
        our_pid,
        mut state,
        mut own_tail,
        mut child_tails,
    } = boot;

    // Per-node count of consecutive ticks a node has presented the
    // `TmuxGone` half-state. §7.5 requires half-states to "resolve via
    // short retry then commit to dead" — we only synthesize a terminal
    // report once the streak crosses `HALF_STATE_TICKS`. Unambiguous
    // `Dead` / `Recycled` verdicts are committed immediately.
    let mut half_state_streak: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();

    // Per-node bounded auto-retry parks (issue `autoretry-agent-died-worker`).
    // A node parked here after an empty-handed `agent-died` is re-spawned once its
    // backoff elapses; the DURABLE bound is `Node.retry_attempts`, so this map is
    // in-memory only and a restart re-derives parks from the persisted count.
    let mut retry_states: std::collections::BTreeMap<String, RetryPark> =
        std::collections::BTreeMap::new();

    // Per-node cumulative-CPU-time tracker for the idle-unmerged safety net's
    // third activity clock (issue `agent-skips-run-merge-idle-pending`). Maps a
    // node id to `(last observed CPU centiseconds, Unix time of the last change)`
    // so `cpu_activity_clock` can tell a busy-but-silent agent (CPU advancing)
    // from a genuinely idle one (CPU flat). In-memory only: on a restart the
    // baseline re-seeds to "active now", conservatively delaying the net by at
    // most one idle window rather than mis-firing.
    let mut cpu_activity: std::collections::BTreeMap<String, (u64, i64)> =
        std::collections::BTreeMap::new();

    // Per-node `pipe-pane` failure counter driving bounded capture retry
    // (issue `capture-agent-output-to-run-dir`). In-memory only: a restart
    // retries from scratch, which is fine. The DURABLE "already armed" set lives
    // in `state.captured_armed` (persisted every tick like `spawned_children`),
    // so a restart does not re-run `pipe-pane` on a still-live capture pipe.
    let mut capture_attempts: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();

    // Repair state written by the pre-state-machine version, which inserted
    // unconfirmed children at pid 0 (the very bug this change fixes). Drop those
    // sentinels so an already-affected run does not treat a never-started child
    // as "confirmed running" forever — it re-enters the startup state machine
    // via the reseed below and is retried (issue
    // `child-supervisor-spawn-unconfirmed-no-retry`).
    state.spawned_children.retain(|_, pid| *pid != 0);

    // In-flight child-supervisor startup state machine
    // (issue `child-supervisor-spawn-unconfirmed-no-retry`). A child is tracked
    // here from the moment we fork it until its identity-verified pid confirms
    // (then it graduates into the durable `spawned_children` set) or it exhausts
    // its bounded retry budget. Held in memory only: a genuinely-running child
    // keeps its own detached process across a parent restart, so the durable
    // truth is the child's own pid file, not this map — which keeps it out of
    // `SupervisorState` serialization.
    //
    // Seeded on boot from the reseeded tail set so a restart recovers startup
    // tracking (`child.spawned` fires only once and the own tail resumes past
    // it, so a forked-but-not-confirmed child would otherwise be orphaned — the
    // original never-retried bug on the restart path). On a fresh run
    // `child_tails` is empty here, so this is a no-op.
    let mut child_spawns = reseed_child_spawns(
        &root,
        child_tails.keys().map(String::as_str),
        &state,
        Instant::now(),
    );

    // Consecutive ticks our run's manifest.json has been missing. Reset
    // to 0 on any tick where it exists; once it crosses
    // `SELF_TERMINATE_TICKS` we self-terminate (run dir vanished).
    let mut manifest_missing_streak: u32 = 0;

    // Consecutive ticks the run has presented "no worker node and no children"
    // while non-terminal. Once it crosses `NO_WORKER_TICKS` we terminalize the
    // run as `failed` rather than poll forever (see `NO_WORKER_TICKS`). Reset
    // to 0 on any tick where a node or child exists.
    let mut no_worker_streak: u32 = 0;
    // Set once the no-worker guard has terminalized the run, so the loop-exit
    // reason reports the spawn failure honestly instead of `work-complete`.
    let mut spawn_failed_terminal = false;

    // Whether terminal-transition cleanup (tmux window close + worktree
    // remove + branch delete, autonomous kinds only) has already run this
    // process. Performed once when the run is first observed terminal; the
    // steps are idempotent/lenient anyway, but the flag avoids re-shelling
    // out every tick between the terminal transition and the loop exit.
    let mut cleaned = false;

    // Whether the terminal-completion notification hook (`run create --notify`)
    // has been settled (fired, already-fired, or none registered). Tracked
    // SEPARATELY from `cleaned` so a transient marker-append failure does not
    // permanently drop the notification: `cleaned` guards the one-shot teardown
    // (which is fine to do once), but a failed notify must be retried on a later
    // tick. Bounded by `notify_attempts` so a persistent failure (disk full)
    // still lets the supervisor exit rather than spinning forever.
    let mut notified = false;
    let mut notify_attempts: u32 = 0;

    // Rate-limit state for the periodic lossy-drop warning (see
    // `maybe_warn_dropped`). The supervisor renders no envelope mid-run, so
    // this is its only channel for surfacing dropped log events.
    let mut last_dropped_warned: u64 = 0;
    let mut last_dropped_warn_at: Option<Instant> = None;

    // Quarantine a poisoned `events.jsonl` by default; the operator can opt
    // out (then a corrupt line is only skipped in memory, P2 behavior).
    let quarantine = !args.no_quarantine_corrupt_lines;

    let mut iter: u32 = 0;
    let exit_reason: &'static str = loop {
        if SIGNAL_RECEIVED.load(Ordering::SeqCst) != 0 {
            break "signal";
        }

        // Orphan defense — checked BEFORE any side-effecting work. When
        // our run's manifest has vanished, the run dir was removed out
        // from under us (a test's `TempDir` teardown, or an operator
        // deleting the run). We must NOT proceed into the tail/watchdog/
        // state-save work below: those write through `create_dir_all`
        // (atomic writes + `flock` acquisition) and would resurrect the
        // very directory we've decided is gone, ghost-file by ghost-file,
        // on every tick. Manifest writes are atomic (tempfile + rename),
        // so manifest.json is never transiently absent during a
        // legitimate rewrite — but we still require a short consecutive
        // streak so a one-off `stat` hiccup can't kill a live supervisor.
        match paths.manifest().try_exists() {
            Ok(false) => {
                manifest_missing_streak += 1;
                if manifest_missing_streak >= SELF_TERMINATE_TICKS {
                    break "run-dir-vanished";
                }
                std::thread::sleep(WATCHDOG_TICK);
                continue;
            }
            // Present, or a stat error (permission flip, NFS hiccup) that
            // is not proof the run is gone: reset the streak and keep
            // supervising.
            Ok(true) | Err(_) => manifest_missing_streak = 0,
        }

        if let Some(max) = args.max_iter {
            if iter >= max {
                break "test-bounded-exit";
            }
        }
        iter += 1;

        // Loop 1: own-run events.
        let own_events = match own_tail.poll() {
            Ok(v) => v,
            Err(e) => {
                warn!(target: "orchestratectl::supervise", error = %e.message, "own tail failed");
                Vec::new()
            }
        };
        for ev in own_events {
            state.last_seq_own = ev.seq;
            match ev.kind.as_str() {
                "child.spawned" => {
                    let child_run_id = ev
                        .data
                        .get("child_run_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let Some(child_run_id) = child_run_id else {
                        warn!(
                            target: "orchestratectl::supervise",
                            seq = ev.seq,
                            "child.spawned missing child_run_id; skipping"
                        );
                        continue;
                    };
                    // Validate before using the id to build filesystem
                    // paths — a malformed child_run_id from the event log
                    // must never escape the runs root.
                    let Ok(child_run_id) = parse_run_id(&child_run_id).map(|r| r.to_string())
                    else {
                        warn!(
                            target: "orchestratectl::supervise",
                            seq = ev.seq,
                            child = %child_run_id,
                            "child.spawned has unsafe child_run_id; skipping"
                        );
                        continue;
                    };
                    // The spawning parent node is `ev.node_id` (the CLI
                    // always sets it when it writes child.spawned). Attribute
                    // the child's report-derived items to THIS node — never
                    // fall back to a guessed root node; skip a malformed
                    // event instead.
                    let Some(parent_node_id) = ev.node_id.clone() else {
                        warn!(
                            target: "orchestratectl::supervise",
                            seq = ev.seq,
                            child = %child_run_id,
                            "child.spawned missing node_id; skipping"
                        );
                        continue;
                    };
                    // Always open a tail for the child, independently of
                    // whether the supervisor fork succeeds — the tail is
                    // the primary consumption path, so a spawn failure must
                    // never orphan the child's reports.
                    let child_events = run_paths(&root, &child_run_id)?.events();
                    let seq = state
                        .last_processed_report_seq_by_child
                        .get(&child_run_id)
                        .copied()
                        .unwrap_or(0);
                    child_tails
                        .entry(child_run_id.clone())
                        .or_insert_with(|| ChildTracking {
                            parent_node_id: parent_node_id.to_string(),
                            tail: tail::EventTail::new(child_events, seq),
                            terminal: false,
                        });
                    // Fork the child supervisor exactly once (the parent's
                    // tracking sets are the single arbiter, §7.2): a child is
                    // either confirmed-running (`spawned_children`) or in-flight
                    // in the startup state machine (`child_spawns`). We NEVER
                    // record a child as started at pid 0 — an unconfirmed boot
                    // stays `Starting` and is promoted only on an
                    // identity-verified pid, or retried under a bounded policy
                    // (issue `child-supervisor-spawn-unconfirmed-no-retry`).
                    if state.spawned_children.contains_key(&child_run_id)
                        || child_spawns.contains_key(&child_run_id)
                    {
                        continue;
                    }
                    match fork_child_supervisor(&root, &child_run_id) {
                        Ok(()) => {
                            // Forked, not yet confirmed. The reconcile pass
                            // polls the child's pid file on later ticks and
                            // promotes it to `spawned_children` only on an
                            // identity-verified pid.
                            child_spawns.insert(
                                child_run_id.clone(),
                                ChildSpawn::Starting {
                                    since: Instant::now(),
                                    attempts: 1,
                                },
                            );
                        }
                        Err(e) => {
                            warn!(
                                target: "orchestratectl::supervise",
                                child = %child_run_id,
                                error = %e.message,
                                "child fork failed (tail still open; will retry under bounded policy)"
                            );
                            // Record on parent log so a future operator can see
                            // the failure (D1), then schedule a bounded retry
                            // rather than dropping the child — a fork failing
                            // (transient EAGAIN / PTY exhaustion) is exactly what
                            // the retry policy exists to ride out.
                            let _ = append_and_apply_event(
                                &paths,
                                "child.spawn_failed",
                                ev.node_id.as_ref(),
                                None,
                                json!({
                                    "child_run_id": child_run_id,
                                    "reason": e.message,
                                    "attempts": 1,
                                }),
                            );
                            child_spawns.insert(
                                child_run_id.clone(),
                                ChildSpawn::Failed {
                                    attempts: 1,
                                    retry_at: Instant::now() + child_retry_backoff(1),
                                },
                            );
                        }
                    }
                }
                "run.status" => {
                    if let Some(s) = ev.data.get("status").and_then(Value::as_str) {
                        if matches!(s, "done" | "failed" | "cancelled") {
                            // Terminal status on our own run is the signal
                            // that we should wind down. We don't break here:
                            // wind-down is driven by `all_work_done` at the
                            // bottom of the tick (which re-reads the manifest
                            // and also waits for any non-terminal children).
                            // Persist the cursor so the decision survives a
                            // crash before that check runs.
                            let _ = state::save(&paths.root, &state);
                        }
                    }
                }
                _ => {}
            }
        }
        // If the own-run tail stopped at a corrupt line, heal (quarantine) or
        // skip past it so the tail keeps progressing.
        report_corrupt_line(&mut own_tail, &paths, &paths, quarantine, "own");

        // Child-supervisor startup reconcile (issue
        // `child-supervisor-spawn-unconfirmed-no-retry`). Each tracked child is
        // in the `Starting`/`Failed` state machine until its identity-verified
        // pid confirms (→ `spawned_children`, dropped from here) or its retry
        // budget is exhausted. Runs every tick: the `child.spawned` event that
        // seeded it fired only once, so promotion + bounded retry must live on
        // the poll path, not the event path.
        reconcile_child_spawns(&root, &paths, &mut child_spawns, &mut state, Instant::now());

        // Loop 2: child-run events.
        let child_ids: Vec<String> = child_tails.keys().cloned().collect();
        for cid in child_ids {
            let entry = child_tails.get_mut(&cid).unwrap();
            if entry.terminal {
                continue;
            }
            let evs = match entry.tail.poll() {
                Ok(v) => v,
                Err(e) => {
                    warn!(
                        target: "orchestratectl::supervise",
                        child = %cid,
                        error = %e.message,
                        "child tail failed"
                    );
                    continue;
                }
            };
            for ev in evs {
                state.last_seq_by_child.insert(cid.clone(), ev.seq);
                match ev.kind.as_str() {
                    "node.report" => {
                        let child_node_id = ev
                            .node_id
                            .as_ref()
                            .map_or("n-0001", NodeId::as_str)
                            .to_string();
                        let parent_node_id = entry.parent_node_id.clone();
                        match reducer::process_node_report(
                            &paths,
                            &parent_node_id,
                            &cid,
                            &child_node_id,
                            ev.seq,
                            &ev.data,
                            &mut state,
                        ) {
                            Ok(Some(c)) => {
                                info!(
                                    target: "orchestratectl::supervise",
                                    child = %cid,
                                    seq = ev.seq,
                                    discussions = c.emitted_discussions.len(),
                                    spinoffs = c.emitted_spinoffs.len(),
                                    skipped = c.skipped_already_present,
                                    "consumed node.report"
                                );
                                entry.terminal = true;
                            }
                            Ok(None) => {
                                // Already processed (cursor replay guard).
                                entry.terminal = true;
                            }
                            Err(e) => {
                                // Consumption failed (transient IO / lock).
                                // Do NOT terminalize and do NOT advance the
                                // durable cursor — rewind this tail to just
                                // before THIS report's seq so the report (and
                                // only it onward) is retried on a later tick
                                // instead of being silently lost
                                // (at-least-once). Re-emitting already-written
                                // discussions/spinoffs is safe: the reducer
                                // skips any deterministic ID already on disk.
                                warn!(
                                    target: "orchestratectl::supervise",
                                    child = %cid,
                                    seq = ev.seq,
                                    error = %e.message,
                                    "node.report consumption failed; will retry"
                                );
                                let rewind_to = ev.seq.saturating_sub(1);
                                let p = entry.tail.path().to_path_buf();
                                entry.tail = tail::EventTail::new(p, rewind_to);
                                // Keep the observational cursor consistent so
                                // it never points past the un-consumed report.
                                state.last_seq_by_child.insert(cid.clone(), rewind_to);
                                entry.terminal = false;
                                break;
                            }
                        }
                    }
                    "run.status" => {
                        if let Some(s) = ev.data.get("status").and_then(Value::as_str) {
                            if matches!(s, "done" | "failed" | "cancelled") {
                                entry.terminal = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A corrupt line in a child's log is reported on our own run log
            // (keyed by source = child id), and quarantined out of the child's
            // own log under that child's lock, so one child's bit rot can't
            // wedge the whole supervisor or strand poison bytes on disk. If the
            // child's paths can't be rebuilt, fall back to an in-memory skip.
            let child_owner = run_paths(&root, &cid).ok();
            let (owner, q) = match child_owner.as_ref() {
                Some(cp) => (cp, quarantine),
                None => (&paths, false),
            };
            report_corrupt_line(&mut entry.tail, &paths, owner, q, &cid);
        }

        // Durable agent-pane capture. Arm `tmux pipe-pane` for any worker node
        // whose `tmux_identity` we can now see, teeing its pane to
        // `<run-dir>/agent.log` so a death remains diagnosable after the window
        // is torn down (issue `worker-process-hang`). Best-effort and additive:
        // runs BEFORE the watchdog so startup output is captured even inside the
        // spawn-grace window, and never blocks liveness (every tmux call is
        // time-bounded). The armed set is persisted in `state`; retries are
        // in-memory.
        capture::capture_tick(&paths, &mut state.captured_armed, &mut capture_attempts);

        // Loop 3: watchdog. We don't yet have a generalized agent
        // registry (that's `all-kinds-spawn`'s territory). The current
        // surface exercises liveness for any node that carries an
        // `agent_pid` recorded by `create.sh` integration.
        if let Err(e) = watchdog_tick(
            &paths,
            &mut half_state_streak,
            &mut retry_states,
            &mut cpu_activity,
        ) {
            warn!(
                target: "orchestratectl::supervise",
                error = %e.message,
                "watchdog tick failed"
            );
        }

        // Fail-loud guard: a non-terminal run with no worker node and no
        // children can never make progress (see `NO_WORKER_TICKS`). Terminalize
        // it as `failed` instead of polling forever or — for a reattached
        // zombie whose manifest a `run cancel` later flips terminal — falsely
        // exiting `work-complete` with zero children spawned (issue
        // `supervisor-spawn-fails-silently-at-run-create`, suggested-fix #5).
        //
        // Two gates keep this from clipping a legitimately in-flight creation
        // (whose `create.sh` may still be materializing the worker for up to
        // the caller's `--agent-startup-timeout`): the streak, AND the run must
        // be older than `no_worker_grace()` (well beyond any create window).
        if !spawn_failed_terminal {
            let unlocked = read_manifest_opt(&paths).ok().flatten();
            let old_enough = |m: &octl_core::Manifest| {
                (Utc::now() - m.created_at)
                    .to_std()
                    .is_ok_and(|age| age >= no_worker_grace())
            };
            let candidate = unlocked.as_ref().is_some_and(|m| {
                !m.status.is_terminal()
                    && m.node_count == 0
                    && child_tails.is_empty()
                    && state.spawned_children.is_empty()
                    && old_enough(m)
            });
            if candidate {
                no_worker_streak += 1;
            } else {
                // A node or child now exists, the manifest is terminal /
                // unreadable, or the run is still inside its create window: it
                // can make progress, so reset.
                no_worker_streak = 0;
            }
            if no_worker_streak >= NO_WORKER_TICKS {
                // The predicate above was evaluated on an UNLOCKED read. Re-take
                // the exclusive run lock and re-verify under it before the
                // destructive terminal append — the sanctioned pattern the
                // watchdog uses for the F15 race (a concurrent `node.created` /
                // `run.status` could have landed since the unlocked read). The
                // deterministic idempotency key makes the append itself at-most-
                // once, but only the re-check prevents failing a now-valid run.
                match RunLock::acquire(&paths.lock()) {
                    Ok(guard) => {
                        let fresh = read_manifest_opt(&paths).ok().flatten();
                        let still_no_worker = fresh.as_ref().is_some_and(|m| {
                            !m.status.is_terminal()
                                && m.node_count == 0
                                && child_tails.is_empty()
                                && state.spawned_children.is_empty()
                        });
                        if still_no_worker {
                            let key = format!("supervisor-no-worker:{run_id}:run-status");
                            let lock = guard.witness();
                            match append_and_apply_unlocked(
                                &lock,
                                &paths,
                                "run.status",
                                None,
                                Some(&key),
                                json!({ "status": "failed", "reason": NO_WORKER_REASON }),
                            ) {
                                Ok(_) => {
                                    warn!(
                                        target: "orchestratectl::supervise",
                                        run_id = %run_id,
                                        reason = NO_WORKER_REASON,
                                        "run has no worker node and no children past the create \
                                         window; terminalizing as failed (supervisor_spawn_failed)"
                                    );
                                    spawn_failed_terminal = true;
                                }
                                Err(e) => {
                                    warn!(
                                        target: "orchestratectl::supervise",
                                        error = %e,
                                        "failed to record no-worker terminal run.status; will retry next tick"
                                    );
                                }
                            }
                        } else {
                            // A node/child/terminal landed under the lock: not a
                            // no-worker run after all. Reset and keep supervising.
                            no_worker_streak = 0;
                        }
                        drop(guard);
                    }
                    Err(e) => {
                        warn!(
                            target: "orchestratectl::supervise",
                            error = %e,
                            "could not lock run to terminalize no-worker; will retry next tick"
                        );
                    }
                }
            }
        }

        // Roll the run up to a terminal status once all of its own nodes —
        // and every tracked child — are terminal. The reducer terminalizes
        // nodes from `node.report` but never the run, so without this an
        // agent's successful terminal report would leave the run `pending`
        // forever and this supervisor polling indefinitely
        // (supervisor-complete-run-on-terminal-report). We are the single
        // arbiter of our run's lifecycle, so — like `run cancel` — we append
        // the `run.status`, here under a deterministic idempotency key so the
        // per-tick re-evaluation appends at most once and a racing cancel is a
        // clean no-op (its terminal manifest makes `rollup_status` return None).
        let children_all_terminal = child_tails.values().all(|t| t.terminal);
        if let Some(status) = cleanup::rollup_status(&paths, children_all_terminal) {
            let status_str = match status {
                Status::Done => "done",
                // Failed is the only other value `rollup_status` returns.
                _ => "failed",
            };
            let key = format!("supervisor-rollup:{run_id}:run-status");
            if let Err(e) = append_and_apply_event(
                &paths,
                "run.status",
                None,
                Some(&key),
                json!({ "status": status_str }),
            ) {
                warn!(
                    target: "orchestratectl::supervise",
                    error = %e,
                    "failed to record terminal run.status; will retry next tick"
                );
            } else {
                info!(
                    target: "orchestratectl::supervise",
                    run_id = %run_id,
                    status = status_str,
                    "rolled run up to terminal status from terminal node(s)"
                );
            }
        }

        // Terminal-transition cleanup: once the run is terminal, close each
        // node's tmux window, remove its worktree, and delete its branch so the
        // run tears itself fully down (supervisor-close-tmux-on-terminal).
        // Cleanup fires when the kind is autonomous (a fire-and-forget run
        // always self-destructs) OR an interactive kind reached terminal via an
        // explicit `run merge` (the user ran the merge, so the review window may
        // close — issue `bundle-worktree-merge`). A plain interactive run that
        // ended without an explicit merge is left alone — the human owns that
        // window. This runs the same tick the rollup above (or a `run cancel`)
        // made the manifest terminal, since `append_and_apply_event` folded it
        // before this read.
        if !cleaned || !notified {
            if let Ok(Some(m)) = read_manifest_opt(&paths) {
                if m.status.is_terminal() {
                    // Fire the completion-notification hook (if any) FIRST —
                    // before teardown removes the worktree/window — so a
                    // spawning session is told the run settled even for a run
                    // whose cleanup is not warranted (a plain interactive run
                    // that ended without an explicit merge). At-least-once,
                    // deduped on a durable `run.notified` marker
                    // (`no-completion-notification-to-parent`). Retried across
                    // ticks on a transient failure (tracked via `notified`,
                    // separate from `cleaned`), bounded by `notify_attempts`.
                    if !notified {
                        notify_attempts += 1;
                        notified = notify::maybe_fire(
                            &paths,
                            &run_id,
                            m.notify_cmd.as_deref(),
                            m.status,
                            crate::run::kind_kebab(m.kind),
                            &m.title,
                        );
                        if !notified && notify_attempts >= NOTIFY_MAX_ATTEMPTS {
                            warn!(
                                target: "orchestratectl::supervise",
                                run_id = %run_id,
                                attempts = notify_attempts,
                                "giving up on completion notify hook after repeated marker-append failures"
                            );
                            // Stop retrying so the run can wind down; the miss
                            // is logged. A last-resort drop after a persistent
                            // lock/I-O failure is acceptable — the loop must be
                            // able to terminate.
                            notified = true;
                        }
                    }
                    let warranted = m.kind.lifecycle() == Lifecycle::Autonomous
                        || cleanup::any_node_merged_explicitly(&paths);
                    if !cleaned && warranted {
                        cleanup::cleanup_terminal_nodes(&paths);
                        // After the node windows are closed, tear down the
                        // managed `--headless` session if this run owned one and
                        // it now holds only its synthetic bootstrap shell window
                        // (issue `headless-tmux-session-not-torn-down`). A no-op
                        // for foreground runs and for a session a sibling run is
                        // still working in.
                        cleanup::cleanup_managed_session(&paths);
                    }
                    // Mark done even when cleanup was not warranted so we don't
                    // re-read the manifest every tick until the loop exits.
                    cleaned = true;
                }
            }
        }

        // Surface lossy-mode log drops periodically. A long-lived
        // supervisor under sustained back-pressure could silently shed
        // `error!`/`warn!` events; this is the only place it can flag that
        // (it renders no success envelope until shutdown).
        maybe_warn_dropped(
            crate::cli::dropped_log_events(),
            Instant::now(),
            &mut last_dropped_warned,
            &mut last_dropped_warn_at,
        );

        // Persist cursors after each tick so a crash mid-run loses at
        // most one tick of progress (and the deterministic-ID reducer
        // makes that loss idempotent anyway). `state::save` is
        // non-creating: if the run dir was deleted mid-tick the write
        // fails harmlessly instead of resurrecting the directory.
        let _ = state::save(&paths.root, &state);

        if args.once {
            break "test-bounded-exit";
        }

        // Cheap idle check: if our run is terminal AND no child
        // remains non-terminal, we're done — but not while a registered
        // completion-notify hook is still pending (`notified` false), so a
        // transient marker-append failure gets its retry ticks before the loop
        // exits. `notify_attempts` bounds this, so a persistent failure still
        // lets us leave.
        if notified && all_work_done(&paths, &child_tails) {
            // Report the spawn failure honestly rather than as `work-complete`
            // when the no-worker guard is what terminalized the run.
            break if spawn_failed_terminal {
                "supervisor-spawn-failed"
            } else {
                "work-complete"
            };
        }

        std::thread::sleep(if iter % 2 == 0 {
            TAIL_TICK
        } else {
            WATCHDOG_TICK
        });
    };

    // Clean shutdown. Persist final cursors. `state::save` is
    // non-creating, so when the run dir vanished this write fails
    // harmlessly rather than resurrecting the deleted directory.
    let _ = state::save(&paths.root, &state);
    let signal_num = SIGNAL_RECEIVED.load(Ordering::SeqCst);
    let signal_name = match signal_num {
        libc::SIGINT => Some("SIGINT"),
        libc::SIGTERM => Some("SIGTERM"),
        _ => None,
    };
    if exit_reason == "run-dir-vanished" {
        warn!(
            target: "orchestratectl::supervise",
            run_id = %run_id,
            pid = our_pid,
            "run dir vanished; supervisor self-terminating"
        );
        // Decisive whole-tree shutdown: SIGTERM every tracked child supervisor
        // before we exit. The common case is already self-healing — each
        // child's run dir lives under the same root and vanishes with ours, so
        // each child self-terminates within ~3s independently — but a child
        // blocked on a lock or mid-`CHILD_DIR_WAIT` could outlive us. Signal
        // it directly rather than relying on every level's independent
        // self-terminate. Signal the UNION of children we forked this process
        // (`spawned_children`) and children reseeded from projections on boot
        // (`child_tails`) — after a crash-restart the former is empty.
        let child_ids: std::collections::BTreeSet<&str> = state
            .spawned_children
            .keys()
            .map(String::as_str)
            .chain(child_tails.keys().map(String::as_str))
            .collect();
        signal_children_term(&root, child_ids.into_iter());
        // Emit a self-terminate marker only if the events log still
        // exists. When the whole run dir was removed (the common case)
        // there is nothing to append to — and we must NOT recreate the
        // directory we just decided is gone. `supervisor.exited` is
        // intentionally skipped here: the dedicated event is clearer for
        // operators reading the log of a still-partially-present run.
        if paths.events().exists() {
            let _ = append_and_apply_event(
                &paths,
                "supervisor.self-terminated",
                None,
                None,
                json!({"pid": our_pid, "reason": "run-dir-vanished"}),
            )
            .map_err(from_core);
        }
    } else {
        let exited_data = match signal_name {
            Some(name) => json!({"pid": our_pid, "reason": "signal", "signal": name}),
            None => json!({"pid": our_pid, "reason": exit_reason}),
        };
        let _ = append_and_apply_event(&paths, "supervisor.exited", None, None, exited_data)
            .map_err(from_core);
    }
    pid_file::remove_if_owner(&pid_path, our_pid);

    #[derive(Serialize)]
    struct ExitedPayload<'a> {
        run_id: &'a str,
        pid: u32,
        reason: &'a str,
        iterations: u32,
    }
    let payload = ExitedPayload {
        run_id: &run_id,
        pid: our_pid,
        reason: exit_reason,
        iterations: iter,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!(
                "supervisor exited run={run_id} pid={our_pid} reason={exit_reason} iter={iter}"
            );
            output::emit_text_warnings(warnings);
        }
    }

    // §7.8: a signal-terminated supervisor exits 130 (SIGINT) / 143
    // (SIGTERM), not 0, so wrappers/tests can detect signal termination.
    // We've already flushed the exit event, removed the PID file, and
    // emitted output, so bypassing destructors here is safe — but flush
    // stdout explicitly first, since `process::exit` skips the buffered
    // writer's drop.
    if signal_num != 0 {
        use std::io::Write as _;
        // Operational breadcrumb: record *why* the supervisor stopped in the
        // process log. `supervisor.exited` lives in events.jsonl, so an
        // operator scanning only the JSONL tracing log would otherwise see the
        // supervisor just go silent. (It also doubles as the
        // last-event-before-flush that the SIGTERM-flush test asserts on, but
        // it earns its place on operational grounds alone.)
        info!(
            target: "orchestratectl::supervise",
            run_id = %run_id,
            pid = our_pid,
            signal = signal_name.unwrap_or("unknown"),
            "supervisor received termination signal; flushing logs and exiting"
        );
        // `process::exit` bypasses the `LogGuard`'s `Drop`, so the buffered
        // tracing events this supervisor emitted (boot + per-tick + the line
        // above) would be lost. Drain them to disk first — the same
        // flush-on-exit contract `event tail`'s signal path uses (see
        // `issues/log-guard-flush-on-process-exit`). Done after the
        // envelope/stdout so it is the last act before exiting.
        let _ = std::io::stdout().flush();
        crate::cli::flush_logs();
        let code = if signal_num == libc::SIGINT { 130 } else { 143 };
        std::process::exit(code);
    }
    Ok(())
}

/// SIGTERM every tracked child supervisor for a decisive whole-tree
/// shutdown when our run dir vanished. Best-effort: errors (a child that
/// already exited, an unreadable record) are ignored. We only signal a pid
/// whose identity we can verify against the child's own `supervisor.pid`
/// record (start-time check, §7.6), so a recycled PID now owned by an
/// unrelated process is never signalled.
///
/// `child_run_ids` must be the UNION of `state.spawned_children` (children we
/// forked this process) and the live `child_tails` keys (children reseeded
/// from projections on boot). After a crash-restart `spawned_children` is
/// empty, so iterating it alone would silently orphan every adopted child.
fn signal_children_term<'a>(root: &Path, child_run_ids: impl Iterator<Item = &'a str>) {
    for child_run_id in child_run_ids {
        let Ok(child_paths) = run_paths(root, child_run_id) else {
            continue;
        };
        // The child wrote its current pid (and start-time) into its own
        // supervisor.pid under the run flock; a child blocked on a lock — the
        // case worth signalling — still has a live run dir and a readable
        // record. If the whole child run dir vanished too, there is no record
        // and nothing to signal (that child self-terminates like we did).
        let Some((pid, start_time)) = pid_file::read_pid_record(&child_paths.supervisor_pid())
        else {
            continue;
        };
        // `read_pid_record` already rejects out-of-range pids; re-narrow here
        // so the `kill` cast can never become a negative process-group target.
        let Some(pid_t) = pid_file::to_pid_t(pid) else {
            continue;
        };
        if !pid_file::pid_live_with_identity(pid, start_time) {
            continue;
        }
        // SAFETY: `kill` with a real signal to a range-checked pid whose
        // identity we just verified; ESRCH (it exited meanwhile) is ignored.
        unsafe {
            libc::kill(pid_t, libc::SIGTERM);
        }
        info!(
            target: "orchestratectl::supervise",
            child = %child_run_id,
            pid,
            "sent SIGTERM to child supervisor (parent shutting down on run-dir-vanished)"
        );
    }
}

struct ChildTracking {
    /// The parent node (in *our* run) that spawned this child — captured
    /// from `child.spawned`'s `node_id` (or the node projection on
    /// reseed). This is where the child's report-derived discussions /
    /// spinoffs are attributed; we must never guess it.
    parent_node_id: String,
    tail: tail::EventTail,
    terminal: bool,
}

/// Ephemeral startup state of a child supervisor this process forked. A child
/// is tracked from the moment we fork it until its identity-verified pid
/// confirms (→ graduates into the durable `spawned_children` set) or its
/// bounded retry budget is exhausted. Held only in memory for the tick loop —
/// see the `child_spawns` declaration for why it is never persisted.
#[derive(Debug, Clone)]
enum ChildSpawn {
    /// Forked; polling the child's `supervisor.pid` for an identity-verified
    /// pid. `since` anchors the [`CHILD_SPAWN_DEADLINE`]; `attempts` counts boot
    /// attempts so far (the initial fork is attempt 1).
    Starting { since: Instant, attempts: u32 },
    /// The last attempt's deadline expired (or its fork failed) without a
    /// confirmed pid. Waits until `retry_at` before re-forking, unless
    /// `attempts` has reached [`CHILD_SPAWN_MAX_ATTEMPTS`].
    Failed { attempts: u32, retry_at: Instant },
}

/// The action [`reconcile_child_spawns`] should take for one tracked child,
/// given a fresh non-blocking read of its identity-verified pid. Pure decision
/// — no I/O — so the startup state machine is unit-testable with an injected
/// clock.
#[derive(Debug, PartialEq, Eq)]
enum SpawnAction {
    /// An identity-verified pid is readable: record the attach and graduate the
    /// child into the durable `spawned_children` set (never at pid 0).
    Confirm(u32),
    /// The `Starting` deadline expired without a pid: emit `child.spawn_failed`
    /// and move to `Failed` with a backoff.
    MarkFailed,
    /// `Failed`, the backoff elapsed, and attempts remain: re-fork.
    Retry,
    /// Nothing to do this tick (still inside the deadline, backing off, or the
    /// retry budget is exhausted).
    Wait,
}

/// Decide what to do with a tracked child this tick. An identity-verified pid
/// always wins — even a boot we had given up waiting for, finally coming up, is
/// adopted rather than re-forked (a second live supervisor for one run is worse
/// than a slow one). Crucially, the absence of a pid NEVER yields a "confirmed"
/// verdict: an unconfirmed child is held (`Wait`) or retried, never recorded as
/// started at pid 0 (issue `child-supervisor-spawn-unconfirmed-no-retry`).
fn child_spawn_action(
    st: &ChildSpawn,
    confirmed_pid: Option<u32>,
    now: Instant,
    deadline: Duration,
    max_attempts: u32,
) -> SpawnAction {
    if let Some(pid) = confirmed_pid {
        return SpawnAction::Confirm(pid);
    }
    match st {
        ChildSpawn::Starting { since, .. } => {
            if now.saturating_duration_since(*since) >= deadline {
                SpawnAction::MarkFailed
            } else {
                SpawnAction::Wait
            }
        }
        ChildSpawn::Failed { attempts, retry_at } => {
            if *attempts >= max_attempts {
                SpawnAction::Wait
            } else if now >= *retry_at {
                SpawnAction::Retry
            } else {
                SpawnAction::Wait
            }
        }
    }
}

/// Bounded exponential backoff before the retry that follows `attempts` failed
/// boots (`attempts` ≥ 1): `BASE * 2^(attempts-1)`, capped at
/// [`CHILD_RETRY_MAX_BACKOFF`]. The shift is clamped so the doubling can never
/// overflow `Duration`.
fn child_retry_backoff(attempts: u32) -> Duration {
    let shift = attempts.saturating_sub(1).min(5);
    (CHILD_RETRY_BASE_BACKOFF * (1u32 << shift)).min(CHILD_RETRY_MAX_BACKOFF)
}

/// The `attempts` value carried by either variant (used when logging / building
/// the successor state without matching twice at the call site).
fn child_spawn_attempts(st: &ChildSpawn) -> u32 {
    match st {
        ChildSpawn::Starting { attempts, .. } | ChildSpawn::Failed { attempts, .. } => *attempts,
    }
}

/// Rebuild in-flight startup tracking after a restart. For each known child
/// (`child_ids`, the reseeded tail set) that is neither confirmed-running in
/// `state.spawned_children` nor already terminal, seed `Starting` so the next
/// [`reconcile_child_spawns`] pass ADOPTS a survivor via its identity-verified
/// pid (`Confirm`, no fork) and only re-forks a genuinely dead one — making this
/// recovery double-fork-safe. A terminal child needs no supervisor, so it is
/// skipped (never re-spawned). `now` is injected for testability.
fn reseed_child_spawns<'a>(
    root: &Path,
    child_ids: impl Iterator<Item = &'a str>,
    state: &state::SupervisorState,
    now: Instant,
) -> std::collections::BTreeMap<String, ChildSpawn> {
    let mut out = std::collections::BTreeMap::new();
    for cid in child_ids {
        if state.spawned_children.contains_key(cid) {
            continue;
        }
        let child_terminal = run_paths(root, cid)
            .ok()
            .and_then(|cp| read_manifest_opt(&cp).ok().flatten())
            .is_some_and(|m| m.status.is_terminal());
        if child_terminal {
            continue;
        }
        out.insert(
            cid.to_string(),
            ChildSpawn::Starting {
                since: now,
                attempts: 1,
            },
        );
    }
    out
}

/// Drive the child-supervisor startup state machine one tick. For each tracked
/// child, take a SINGLE non-blocking, identity-verified pid read and apply the
/// resulting [`SpawnAction`]:
///   - `Confirm(pid)` — record the attach on both logs and graduate the child
///     into the durable `spawned_children` set (never at pid 0), then drop it.
///   - `MarkFailed` — the `Starting` deadline expired: emit `child.spawn_failed`
///     and back off before the next attempt.
///   - `Retry` — the backoff elapsed and attempts remain: re-fork.
///   - `Wait` — still booting, backing off, or the budget is exhausted.
///
/// `now` is injected so the transitions are testable without real time.
fn reconcile_child_spawns(
    root: &Path,
    parent_paths: &RunPaths,
    child_spawns: &mut std::collections::BTreeMap<String, ChildSpawn>,
    state: &mut state::SupervisorState,
    now: Instant,
) {
    let deadline = child_spawn_deadline();
    // Snapshot the keys so the map can be mutated inside the loop.
    let cids: Vec<String> = child_spawns.keys().cloned().collect();
    for cid in cids {
        // Non-blocking, identity-verified read of the child's own pid file.
        // `None` (no file, or a recycled/mismatched pid) means "not confirmed"
        // — never treated as a successful start.
        let confirmed_pid = run_paths(root, &cid)
            .ok()
            .and_then(|cp| crate::run::supervisor_spawn::read_live_recorded_pid(&cp));
        let attempts = child_spawn_attempts(&child_spawns[&cid]);
        match child_spawn_action(
            &child_spawns[&cid],
            confirmed_pid,
            now,
            deadline,
            CHILD_SPAWN_MAX_ATTEMPTS,
        ) {
            SpawnAction::Confirm(pid) => {
                record_child_attached(root, &cid, parent_paths, pid);
                state.spawned_children.insert(cid.clone(), pid);
                child_spawns.remove(&cid);
                info!(
                    target: "orchestratectl::supervise",
                    child = %cid,
                    pid,
                    attempts,
                    "child supervisor confirmed running"
                );
            }
            SpawnAction::MarkFailed => {
                // This attempt timed out. If the budget is now spent, the
                // successor `Failed` state will only ever return `Wait` — so
                // this is the final, give-up failure, not a "scheduling retry"
                // one. Report it honestly on both the log line and the durable
                // event (the earlier "scheduling retry" wording lied on the last
                // attempt).
                let exhausted = attempts >= CHILD_SPAWN_MAX_ATTEMPTS;
                if exhausted {
                    warn!(
                        target: "orchestratectl::supervise",
                        child = %cid,
                        attempts,
                        "child supervisor never confirmed a pid; retry budget exhausted, giving up"
                    );
                } else {
                    warn!(
                        target: "orchestratectl::supervise",
                        child = %cid,
                        attempts,
                        "child supervisor did not confirm a pid within the deadline; scheduling retry"
                    );
                }
                let _ = append_and_apply_event(
                    parent_paths,
                    "child.spawn_failed",
                    None,
                    None,
                    json!({
                        "child_run_id": cid,
                        "reason": "no identity-verified pid within CHILD_SPAWN_DEADLINE",
                        "attempts": attempts,
                        "final": exhausted,
                    }),
                );
                child_spawns.insert(
                    cid.clone(),
                    ChildSpawn::Failed {
                        attempts,
                        retry_at: now + child_retry_backoff(attempts),
                    },
                );
            }
            SpawnAction::Retry => {
                let next = attempts + 1;
                match fork_child_supervisor(root, &cid) {
                    Ok(()) => {
                        info!(
                            target: "orchestratectl::supervise",
                            child = %cid,
                            attempt = next,
                            "re-forked child supervisor after a failed boot"
                        );
                        child_spawns.insert(
                            cid.clone(),
                            ChildSpawn::Starting {
                                since: now,
                                attempts: next,
                            },
                        );
                    }
                    Err(e) => {
                        warn!(
                            target: "orchestratectl::supervise",
                            child = %cid,
                            attempt = next,
                            error = %e.message,
                            "child re-fork failed; backing off"
                        );
                        let _ = append_and_apply_event(
                            parent_paths,
                            "child.spawn_failed",
                            None,
                            None,
                            json!({
                                "child_run_id": cid,
                                "reason": e.message,
                                "attempts": next,
                            }),
                        );
                        child_spawns.insert(
                            cid.clone(),
                            ChildSpawn::Failed {
                                attempts: next,
                                retry_at: now + child_retry_backoff(next),
                            },
                        );
                    }
                }
            }
            // Still inside the deadline, backing off, or the budget is
            // exhausted. On exhaustion the child stays in `Failed` — its tail
            // stays open so any late report is still consumed — but we stop
            // hammering the fork path. No per-tick log here: the final
            // `child.spawn_failed` (emitted at the last `MarkFailed`) already
            // records the give-up, so logging every tick would only spam.
            SpawnAction::Wait => {}
        }
    }
}

/// Fork+exec a fully-detached child supervisor (setsid + double-fork via
/// `supervisor_spawn`). Returns once the grandchild is launched and the
/// short-lived intermediate is reaped — it does NOT confirm the child's pid.
/// Confirmation is the caller's job on later ticks
/// ([`reconcile_child_spawns`]): the grandchild is reparented to init, so an
/// exited child supervisor never becomes a zombie under this long-lived parent
/// (and `kill(pid, 0)` never misreports a zombie as alive, which would corrupt
/// the PID-staleness check).
fn fork_child_supervisor(root: &Path, child_run_id: &str) -> Result<(), CliError> {
    // D1: tolerate the race window — wait up to CHILD_DIR_WAIT for the
    // child run dir to appear before deciding the spawn has failed.
    // `child_run_id` was validated by the caller; re-parse to feed run_dir a
    // typed RunId (run_dir no longer accepts a raw &str).
    let child_rid = parse_run_id(child_run_id)?;
    let child_dir = octl_core::run_dir(root, &child_rid);
    let deadline = Instant::now() + CHILD_DIR_WAIT;
    while !child_dir.join("manifest.json").exists() {
        if Instant::now() >= deadline {
            return Err(CliError::system(
                "child_dir_missing",
                format!(
                    "child run dir {} did not appear within {:?}",
                    child_dir.display(),
                    CHILD_DIR_WAIT
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // `RUST_LOG_NOSPAWN` is cleared because it is reserved for tests.
    let stderr_path: PathBuf = child_dir.join("supervisor.stderr.log");
    // Child-spawn is lenient: the parent supervisor reads the child's own pid
    // file (non-blocking, on later ticks) rather than confirming over a
    // readiness pipe, so no readiness fd is threaded in.
    let mut cmd =
        crate::run::supervisor_spawn::detached_supervise_command(child_run_id, &stderr_path, None)?;
    cmd.env_remove("RUST_LOG_NOSPAWN");
    crate::run::supervisor_spawn::spawn_and_reap(&mut cmd, child_run_id)?;
    info!(
        target: "orchestratectl::supervise",
        child = %child_run_id,
        "forked child supervisor (pid confirmation deferred to reconcile)"
    );
    Ok(())
}

/// Record a confirmed child supervisor's identity-verified `pid` on BOTH logs:
/// `supervisor.attached` on the CHILD run (folded onto its root node's
/// `supervisor_pid`, so a from-scratch projection rebuild reproduces the field
/// — issue `supervisor-state-not-event-sourced`) and `child.supervisor_attached`
/// on the PARENT run. Called only with a real, live pid — never 0, which would
/// be a false "attached". Best-effort: a projection/append hiccup is logged,
/// not fatal (the durable truth is the child's own pid file).
fn record_child_attached(root: &Path, child_run_id: &str, parent_paths: &RunPaths, pid: u32) {
    // Record `supervisor_pid` on the child's root node (best-effort). If the
    // child paths cannot even be resolved, skip the child append but STILL emit
    // the parent record below — the two are independent, and the parent event is
    // the documented fallback (never short-circuit past it).
    match run_paths(root, child_run_id) {
        Ok(child_paths) => {
            // `append_and_apply_event` takes the child run's flock itself (F11),
            // so this read-modify-write no longer races the child supervisor's
            // own boot writes. The child run's root node is always `n-0001` (a
            // static, valid id).
            let root_node = NodeId::parse_str("n-0001").expect("n-0001 is a valid node id");
            if let Err(e) = append_and_apply_event(
                &child_paths,
                "supervisor.attached",
                Some(&root_node),
                None,
                json!({ "pid": pid }),
            ) {
                // Non-fatal: the `child.supervisor_attached` event below records
                // the attach on the parent log, so the child projection update
                // is a convenience the next reattach can re-derive.
                warn!(
                    target: "orchestratectl::supervise",
                    child = %child_run_id,
                    error = %e,
                    "could not record supervisor.attached on child run"
                );
            }
        }
        Err(e) => warn!(
            target: "orchestratectl::supervise",
            child = %child_run_id,
            error = %e.message,
            "could not resolve child run paths to record supervisor.attached (parent record still emitted)"
        ),
    }
    // Record the attach on the parent log, but only with a real pid — never emit
    // `supervisor_pid: 0`, which would be a false "attached".
    if let Err(e) = append_and_apply_event(
        parent_paths,
        "child.supervisor_attached",
        None,
        None,
        json!({"child_run_id": child_run_id, "supervisor_pid": pid}),
    ) {
        warn!(
            target: "orchestratectl::supervise",
            child = %child_run_id,
            error = %e,
            "could not record child.supervisor_attached on parent run"
        );
    }
}

/// Scan our own `nodes/` for every `child_run_id -> parent_node_id`
/// mapping recorded in a node's `children` list. This is the canonical
/// source for which children this run owns and which local node spawned
/// each — used to (re)seed child tails on boot (§7.6). The first node
/// that lists a given child wins (a child is registered under exactly
/// one node).
fn discover_children(paths: &RunPaths) -> std::collections::BTreeMap<String, String> {
    // Scan every node under the run's shared lock so a concurrent reducer
    // mutating the `nodes/` set cannot hand us a half-updated child map
    // (design.md §4). A lock-acquire failure degrades to an empty map, the same
    // way an unreadable `nodes/` already does.
    RunLock::with_shared_lock(&paths.lock(), || {
        let mut out = std::collections::BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(paths.nodes_dir()) else {
            return Ok(out);
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(node_id) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            // A stem that is not a well-formed node id can't be one of our
            // projection files; skip it.
            let Ok(nid) = NodeId::parse_str(&node_id) else {
                continue;
            };
            if let Ok(Some(n)) = read_node_opt(paths, &nid) {
                for c in &n.children {
                    // `c.run_id` is a validated `RunId` — the projection would have
                    // failed to deserialize otherwise — so it is already safe to
                    // use as a path component when reseeding child tails.
                    out.entry(c.run_id.to_string())
                        .or_insert_with(|| node_id.clone());
                }
            }
        }
        Ok(out)
    })
    .unwrap_or_default()
}

/// If `tail`'s last [`poll`](tail::EventTail::poll) parked at a corrupt line,
/// react to it on *our own* run log (`own`), regardless of which tail (own or
/// child) hit the bad line.
///
/// With `quarantine` set (the supervisor default), the corrupt line is healed
/// out of `log_owner`'s `events.jsonl`: under that run's lock the original is
/// renamed to `events.jsonl.corrupt-<ts>.bak`, a recovered log is written in
/// its place, the tail is restarted at offset 0 (every byte offset shifted),
/// and a single `supervisor.event_log_quarantined` event is emitted. This
/// makes strict replay survive a poisoned log instead of leaving the bytes on
/// disk for every future reader (corrupt-line-quarantine).
///
/// Without `quarantine` (operator opt-out) — or if the quarantine itself fails
/// — we fall back to the P2 behavior: advance past the line in memory and emit
/// a one-shot `supervisor.event_log_skipped_line` the first time that offset is
/// seen, leaving the poison bytes on disk.
///
/// Combined with the unified physical-reader + validate-before-append fixes, a
/// corrupt middle line should only arise from external tampering or bit rot;
/// when it does, we surface it once and keep tailing rather than re-erroring on
/// the same offset forever (F17).
fn report_corrupt_line(
    tail: &mut tail::EventTail,
    own: &RunPaths,
    log_owner: &RunPaths,
    quarantine: bool,
    source: &str,
) {
    let Some(c) = tail.take_new_corrupt() else {
        return;
    };
    warn!(
        target: "orchestratectl::supervise",
        source = %source,
        byte_offset = c.byte_offset,
        excerpt = %c.line_excerpt,
        quarantine,
        "corrupt event-log line detected; continuing tail"
    );

    if quarantine {
        // Filename-safe basic-ISO stamp (no colons) for the `.bak` sibling.
        let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        match octl_core::quarantine_corrupt_lines(log_owner, &ts) {
            Ok(Some(q)) => {
                // The corrupt bytes are gone and every offset shifted: re-read
                // the healed log from the start. `last_seq` is preserved, so
                // already-consumed events are skipped, not reprocessed.
                tail.restart();
                info!(
                    target: "orchestratectl::supervise",
                    source = %source,
                    backup = %q.backup_path.display(),
                    removed = q.removed_byte_offsets.len(),
                    "quarantined corrupt event-log line(s)"
                );
                if let Err(e) = append_and_apply_event(
                    own,
                    "supervisor.event_log_quarantined",
                    None,
                    None,
                    json!({
                        "backup_path": q.backup_path.display().to_string(),
                        "removed_byte_offsets": q.removed_byte_offsets,
                        "source": source,
                    }),
                ) {
                    warn!(
                        target: "orchestratectl::supervise",
                        source = %source,
                        error = %e,
                        "failed to persist quarantine diagnostic (log already healed)"
                    );
                }
                return;
            }
            Ok(None) => {
                // The corrupt line vanished between the tail's read and the
                // locked re-read (e.g. an external truncate). Nothing to heal;
                // fall through to the skip diagnostic.
            }
            Err(e) => {
                warn!(
                    target: "orchestratectl::supervise",
                    source = %source,
                    error = %e,
                    "quarantine failed; falling back to in-memory skip"
                );
            }
        }
    }

    if let Err(e) = append_and_apply_event(
        own,
        "supervisor.event_log_skipped_line",
        None,
        None,
        json!({
            "byte_offset": c.byte_offset,
            "line_excerpt": c.line_excerpt,
            "source": source,
        }),
    ) {
        // The diagnostic could not be persisted — e.g. the corrupt line is the
        // own-run log's final record, so `recover_last_seq` (called by the
        // append) trips on it. We still advanced past the line in memory to
        // keep the tail progressing; surface the failure rather than silently
        // dropping the only record of it.
        warn!(
            target: "orchestratectl::supervise",
            source = %source,
            byte_offset = c.byte_offset,
            error = %e,
            "failed to persist corrupt-line diagnostic (advanced past it anyway)"
        );
    }
}

/// Decide whether to emit a rate-limited "log events dropped" warning, and
/// emit it if so. Warns only when `current` exceeds the last-warned count
/// (i.e. *new* drops since the previous warning) AND at least
/// [`DROPPED_WARN_INTERVAL`] has elapsed since that warning (or none has been
/// emitted yet). On warn, updates `last_count`/`last_at` and returns `true`.
///
/// `current` and `now` are passed in (rather than read inside) so the
/// rate-limit logic is deterministically unit-testable; production callers
/// pass [`crate::cli::dropped_log_events`] and [`Instant::now`].
///
/// The warning goes to **stderr as well as `tracing::warn!`**: the `warn!`
/// travels through the very lossy appender that is dropping events, so under
/// sustained back-pressure the drop-warning could itself be dropped — the
/// stderr line is the reliable channel that always survives (the supervisor's
/// stderr is captured to `supervisor.stderr.log` when detached).
fn maybe_warn_dropped(
    current: u64,
    now: Instant,
    last_count: &mut u64,
    last_at: &mut Option<Instant>,
) -> bool {
    if current <= *last_count {
        return false;
    }
    // `saturating_duration_since`: `now` is injected, so never panic / wrap if
    // a caller passes a non-monotonic timestamp (production passes monotonic).
    let due = last_at.is_none_or(|t| now.saturating_duration_since(t) >= DROPPED_WARN_INTERVAL);
    if !due {
        return false;
    }
    let newly_dropped = current - *last_count;
    warn!(
        target: "orchestratectl::supervise",
        dropped = current,
        newly_dropped,
        "log events dropped due to buffer overflow (lossy non-blocking appender under sustained back-pressure)"
    );
    // Reliable fallback: the `warn!` above can be dropped by the same lossy
    // channel it reports on, so also emit to stderr, which never routes
    // through the appender.
    eprintln!(
        "warning: {current} log events dropped due to buffer overflow \
         ({newly_dropped} new since last warning)"
    );
    *last_count = current;
    *last_at = Some(now);
    true
}

fn all_work_done(
    paths: &RunPaths,
    child_tails: &std::collections::BTreeMap<String, ChildTracking>,
) -> bool {
    let Ok(Some(m)) = read_manifest_opt(paths) else {
        return false;
    };
    if !matches!(m.status, Status::Done | Status::Failed | Status::Cancelled) {
        return false;
    }
    child_tails.values().all(|t| t.terminal)
}

/// Number of consecutive ticks a `TmuxGone` half-state must persist
/// before the watchdog commits to synthesizing a terminal report (§7.5
/// "resolve via short retry then commit to dead").
const HALF_STATE_TICKS: u32 = 3;

/// The effective spawn grace this process uses: [`WATCHDOG_SPAWN_GRACE`]
/// unless [`SPAWN_GRACE_ENV`] overrides it with a parseable whole-second
/// count (an unparseable value is ignored, keeping the safe default).
fn spawn_grace() -> Duration {
    match std::env::var(SPAWN_GRACE_ENV) {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => WATCHDOG_SPAWN_GRACE,
        },
        Err(_) => WATCHDOG_SPAWN_GRACE,
    }
}

/// Whether a node is still inside its spawn grace window and must therefore
/// be left untouched by the watchdog this tick.
///
/// `true` only when `started_at` is known AND `now` is less than `grace`
/// past it. A node with no `started_at` (legacy / malformed projection) is
/// treated as eligible (`false`): we cannot prove it is fresh, and
/// suppressing forever would be worse than the original race. A clock that
/// went backwards (`now < started_at`) is also treated as in-grace — the
/// node cannot be older than its own creation, so the conservative read is
/// "too fresh to judge".
fn within_spawn_grace(
    started_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    grace: Duration,
) -> bool {
    let Some(started_at) = started_at else {
        return false;
    };
    match (now - started_at).to_std() {
        // `to_std` succeeds only for a non-negative delta; a node younger
        // than `grace` is still in the window.
        Ok(age) => age < grace,
        // Negative delta (clock skew / `now` before creation): conservatively
        // in-grace.
        Err(_) => true,
    }
}

/// In-memory park for a node awaiting a bounded auto-retry re-spawn after an
/// empty-handed `agent-died` (issue `autoretry-agent-died-worker`).
///
/// Ephemeral by design — the DURABLE, restart-safe bound is `Node.retry_attempts`
/// (incremented by each `node.retry` event). This park only holds the backoff
/// deadline, the death reason (for the audit event), and the in-memory
/// `create.sh`-failure counter for the current re-spawn. On a supervisor restart
/// the map is empty; the watchdog simply re-detects the still-dead pid and
/// re-parks from the persisted attempt count, so no reseed is needed and the
/// bound is never exceeded.
struct RetryPark {
    /// 1-based attempt number this park performs — the value `Node.retry_attempts`
    /// will hold AFTER the re-spawn's `node.retry` event. Used for backoff sizing,
    /// the audit event, and logging.
    attempt: u32,
    /// Earliest instant the re-spawn may fire (death instant + backoff).
    retry_at: Instant,
    /// The liveness verdict that killed the previous agent (`agent-died`,
    /// `agent-tmux-window-gone`, `agent-pid-recycled`), carried onto the durable
    /// `node.retry` event so the retry history records WHY each attempt happened.
    reason: String,
    /// Consecutive `create.sh` failures for THIS re-spawn. Bounds a broken spawn
    /// infrastructure (missing create.sh, exhausted PTYs) so the reconcile cannot
    /// loop forever — a distinct failure mode from a dying agent.
    spawn_failures: u32,
}

/// Whether a node is retry-eligible: an autonomous, top-level, single-node worker
/// kind (never interactive, never a DAG child, never a driver). See
/// [`octl_core::Kind::is_autonomous_single_node_worker`].
fn retry_eligible_kind(n: &Node) -> bool {
    n.kind.lifecycle() == Lifecycle::Autonomous
        && n.kind.is_autonomous_single_node_worker()
        && n.parent_node_id.is_none()
}

/// Drive the bounded auto-retry state machine for every parked node whose backoff
/// has elapsed (issue `autoretry-agent-died-worker`).
///
/// For each due park: re-verify (under the run lock) that the node is still a
/// non-terminal, POSITIVELY empty-handed, retry-eligible worker — the retry ⟂
/// salvage guard, so a report that raced in, a `run cancel`, or a late-committing
/// agent drops the park instead of re-spawning over settled/committed work. Then,
/// OUTSIDE the lock (the I/O is slow): tear down the stale worktree and `create.sh`
/// a clean one at the run's source branch. On success, emit a durable `node.retry`
/// event rewiring the node to the new agent (and incrementing the persisted
/// attempt bound). On a spawn-infrastructure failure, back off and reschedule; once
/// that in-memory failure budget is exhausted, terminalize the run `failed`.
fn reconcile_agent_retries(
    paths: &RunPaths,
    retry_states: &mut std::collections::BTreeMap<String, RetryPark>,
    now: Instant,
) {
    // Snapshot the due node ids first, so we never hold an iterator over
    // `retry_states` while mutating it per node below.
    let due: Vec<String> = retry_states
        .iter()
        .filter(|(_, p)| now >= p.retry_at)
        .map(|(k, _)| k.clone())
        .collect();
    if due.is_empty() {
        return;
    }
    let git = cleanup::git_bin();
    let tmux = cleanup::tmux_bin();
    for node_id in due {
        let Ok(nid) = NodeId::parse_str(&node_id) else {
            retry_states.remove(&node_id);
            continue;
        };
        // (1) Under the run lock: read the node and re-verify it still warrants a
        // retry. A terminal node (a report raced in, or `run cancel`) or one that
        // is no longer positively empty-handed (a late commit landed → salvage
        // territory) drops the park; the next watchdog tick handles it.
        let guard = match RunLock::acquire(&paths.lock()) {
            Ok(g) => g,
            Err(e) => {
                warn!(
                    target: "orchestratectl::supervise",
                    node = %node_id, error = %e,
                    "could not lock run to reconcile retry; will retry next tick"
                );
                continue;
            }
        };
        let node = read_node_opt(paths, &nid).ok().flatten();
        let proceed = node.as_ref().is_some_and(|n| {
            !matches!(n.status, Status::Done | Status::Failed | Status::Cancelled)
                && retry_eligible_kind(n)
                && cleanup::node_is_empty_handed(paths, n, &git)
        });
        if !proceed {
            info!(
                target: "orchestratectl::supervise",
                node = %node_id,
                "retry park no longer applies (terminal / not empty-handed); dropping"
            );
            retry_states.remove(&node_id);
            drop(guard);
            continue;
        }
        let node = node.expect("proceed implies Some");
        // Capture everything the re-spawn needs while the lock is held; release it
        // before the slow teardown + create.sh I/O.
        let manifest = read_manifest_opt(paths).ok().flatten();
        drop(guard);

        let Some(manifest) = manifest else {
            warn!(
                target: "orchestratectl::supervise",
                node = %node_id, "manifest unreadable during retry; will retry next tick"
            );
            continue;
        };
        let attempt = retry_states.get(&node_id).map_or(1, |p| p.attempt);
        let reason = retry_states
            .get(&node_id)
            .map_or_else(|| "agent-died".to_string(), |p| p.reason.clone());

        // (2) Spawn the fresh worker FIRST — BEFORE tearing down the stale one.
        // Spawn-before-teardown is the crux of crash/failure safety:
        //   - the stale empty-handed worktree survives a `create.sh` failure, so the
        //     next tick can re-verify empty-handedness and the `spawn_failures`
        //     budget is real (a teardown-first ordering deletes the branch the
        //     re-verify depends on, silently collapsing the budget);
        //   - the fresh worker uses a distinct `-rN` branch name, so it never
        //     collides with the stale one still on disk.
        let outcome = respawn_agent(paths, &node, &manifest, attempt);
        let spawn = match outcome {
            Ok(s) => s,
            Err(e) => {
                // create.sh failed: broken spawn infrastructure, NOT a dying agent.
                // The stale worktree is untouched, so this is a clean retry. Bounded
                // in memory so a host that cannot spawn cannot loop forever.
                let failures = retry_states
                    .get(&node_id)
                    .map_or(1, |p| p.spawn_failures + 1);
                if failures >= agent_respawn_max_failures() {
                    warn!(
                        target: "orchestratectl::supervise",
                        node = %node_id, error = %e.message, failures,
                        "re-spawn failed repeatedly; terminalizing run failed"
                    );
                    // Remove the park ONLY once the terminal report is durably
                    // recorded; otherwise keep it so a transient lock/append failure
                    // re-fires terminalization instead of silently un-tracking the
                    // node (which would let a restart re-park at spawn_failures=0).
                    if terminalize_respawn_failure(paths, &nid, &node_id, attempt, &e.message) {
                        retry_states.remove(&node_id);
                    }
                } else if let Some(park) = retry_states.get_mut(&node_id) {
                    warn!(
                        target: "orchestratectl::supervise",
                        node = %node_id, error = %e.message, failures,
                        "re-spawn failed; backing off and rescheduling"
                    );
                    park.spawn_failures = failures;
                    park.retry_at = now + agent_retry_backoff(attempt);
                }
                continue;
            }
        };

        // (3) The fresh worker exists. Under the lock, RE-VERIFY the stale node is
        // still non-terminal AND still empty-handed, then rewire it to the new
        // agent. Any abort here must tear down the fresh worker (kill + worktree +
        // branch + window) — an unrewired spawn is an orphan that also poisons the
        // `-rN` branch name for the next attempt.
        let base_sha = crate::run::create::capture_base_sha(&spawn.worktree_path);
        let guard = match RunLock::acquire(&paths.lock()) {
            Ok(g) => g,
            Err(e) => {
                warn!(
                    target: "orchestratectl::supervise",
                    node = %node_id, error = %e,
                    "could not lock run to record node.retry; tearing down fresh spawn, will retry"
                );
                teardown_respawn_outcome(&spawn, manifest.source_repo.as_deref(), &tmux, &git);
                continue;
            }
        };
        // Re-read against the SAME criteria as the pre-spawn check. A `run cancel`
        // that raced the spawn (terminal), or a late commit on the stale branch
        // (no longer empty-handed → salvage territory), aborts the retry: the
        // stale node/branch is left intact for the terminal/salvage path, and the
        // fresh spawn is torn down. This is the retry ⟂ salvage guard — a retry can
        // never rewire away from, or clobber, a branch that gained committed work.
        let recheck = read_node_opt(paths, &nid).ok().flatten();
        let still_retryable = recheck.as_ref().is_some_and(|n| {
            !matches!(n.status, Status::Done | Status::Failed | Status::Cancelled)
                && cleanup::node_is_empty_handed(paths, n, &git)
        });
        if !still_retryable {
            let terminal = recheck.as_ref().is_some_and(|n| {
                matches!(n.status, Status::Done | Status::Failed | Status::Cancelled)
            });
            warn!(
                target: "orchestratectl::supervise",
                node = %node_id, terminal,
                "node no longer retryable after re-spawn (terminal or committed work appeared); \
                 tearing down fresh spawn, dropping park"
            );
            drop(guard);
            teardown_respawn_outcome(&spawn, manifest.source_repo.as_deref(), &tmux, &git);
            retry_states.remove(&node_id);
            continue;
        }
        let data = json!({
            "attempt": attempt,
            "reason": reason,
            "branch": spawn.branch,
            "base_sha": base_sha,
            "worktree_path": spawn.worktree_path,
            "tmux_window": spawn.tmux_window,
            "tmux_socket": spawn.tmux_socket,
            "tmux_session": spawn.tmux_session,
            "tmux_window_id": spawn.tmux_window_id,
            "agent_pid": spawn.agent_pid,
        });
        let lock = guard.witness();
        if let Err(e) =
            append_and_apply_unlocked(&lock, paths, "node.retry", Some(&nid), None, data)
        {
            warn!(
                target: "orchestratectl::supervise",
                node = %node_id, error = %e,
                "record node.retry failed; tearing down fresh spawn, leaving park to re-fire"
            );
            drop(guard);
            // Tear down the fresh spawn so the next re-fire spawns cleanly on the
            // same `-rN` name (no orphan, no collision).
            teardown_respawn_outcome(&spawn, manifest.source_repo.as_deref(), &tmux, &git);
            continue;
        }
        drop(guard);

        // (4) The node is durably rewired to the fresh worker. NOW tear down the
        // stale worktree + branch + tmux window (still empty-handed, re-checked by
        // `cleanup_node`'s own source-relative guard, which PRESERVES rather than
        // deletes if commits somehow appeared — never destroying committed work).
        cleanup::cleanup_node(paths, &node, &tmux, &git);
        info!(
            target: "orchestratectl::supervise",
            node = %node_id, attempt, branch = %spawn.branch,
            "re-spawned empty-handed worker on fresh worktree at source branch"
        );
        retry_states.remove(&node_id);
    }
}

/// Best-effort teardown of a fresh re-spawn that could NOT be durably attached to
/// its node (a `run cancel` raced it, a late commit turned the stale node into
/// salvage territory, or the `node.retry` append failed). Kills the new agent,
/// removes its worktree, deletes its `-rN` branch, and closes its tmux window — so
/// an unattached spawn never leaks a live token-burning agent, and its branch name
/// is freed for a clean re-fire (issue `autoretry-agent-died-worker`, from
/// llm-review). Every step is independent and swallows its own errors; a partial
/// teardown still makes progress.
fn teardown_respawn_outcome(spawn: &RespawnOutcome, repo: Option<&str>, tmux: &str, git: &str) {
    use std::process::{Command, Stdio};
    // Kill the agent process first so it stops editing before its worktree is
    // pulled. TERM (not KILL) lets it shut down its own child processes.
    if spawn.agent_pid > 0 {
        let _ = Command::new("kill")
            .arg(spawn.agent_pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    // Close the tmux window (best-effort; a nonexistent window / unavailable tmux
    // is a clean no-op). 1:1 swap of the former raw `tmux kill-window` for the
    // typed multiplexer call; the socket omission is preserved from prior
    // behavior (this retry-teardown targeted the default server).
    if !spawn.tmux_window.is_empty() {
        crate::multiplexer::tmux::Tmux::with_bin(tmux).kill_window(None, &spawn.tmux_window);
    }
    // Remove the worktree, then force-delete the branch, from the run's source repo
    // (`git worktree remove` / `branch -D` operate on the main repo's worktree
    // list, so `-C <repo>` is required — the detached supervisor's cwd is not the
    // repo). The branch was just minted by THIS retry (an `-rN` name) and carries
    // no work worth keeping, so a force delete is safe and intentional — it frees
    // the name for a clean re-fire. Without a known repo we cannot safely target
    // the worktree list, so skip (the next re-fire's create.sh will surface the
    // collision as a spawn failure rather than us guessing a repo).
    let Some(repo) = repo.filter(|s| !s.is_empty()) else {
        return;
    };
    let _ = Command::new(git)
        .args([
            "-C",
            repo,
            "worktree",
            "remove",
            "--force",
            &spawn.worktree_path,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !spawn.branch.is_empty() {
        let _ = Command::new(git)
            .args(["-C", repo, "branch", "-D", "--", &spawn.branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Metadata a successful re-spawn wires onto the node's `node.retry` event.
struct RespawnOutcome {
    branch: String,
    worktree_path: String,
    tmux_window: String,
    tmux_socket: Option<String>,
    tmux_session: Option<String>,
    tmux_window_id: Option<String>,
    agent_pid: i64,
}

/// Default startup window the retry re-spawn gives a fresh agent to become
/// discoverable, matching `run create`'s default (higher than create.sh's own 30s
/// so a loaded host does not fail the re-spawn spuriously).
const AGENT_RESPAWN_STARTUP_TIMEOUT: u32 = 90;

/// Shell out to `create.sh` to materialize a fresh worker at the run's source
/// branch, from the run's source repo, driven by the run's original `prompt.md`.
/// Returns the new agent's spawn coordinates, or a `CliError` on any spawn failure
/// (create.sh error, PID died instantly). Pure I/O — the caller holds no lock.
fn respawn_agent(
    paths: &RunPaths,
    node: &Node,
    manifest: &octl_core::Manifest,
    attempt: u32,
) -> Result<RespawnOutcome, CliError> {
    let prompt_path = paths.root.join("prompt.md");
    if !prompt_path.exists() {
        return Err(CliError::system(
            "respawn_prompt_missing",
            format!(
                "prompt file {} not found for re-spawn",
                prompt_path.display()
            ),
        ));
    }
    // Absolutize the prompt path: `respawn_agent` sets create.sh's cwd to the
    // source repo, so a relative prompt path would otherwise resolve against the
    // repo instead of the run dir. `canonicalize` is safe — the file exists.
    let prompt_path = prompt_path.canonicalize().unwrap_or(prompt_path);
    let source_branch = manifest.source_branch.as_deref();
    let source_repo = manifest.source_repo.as_deref().map(std::path::Path::new);
    let branch = retry_branch_name(node.branch.as_deref(), attempt);
    let req = crate::run::spawn::SpawnRequest {
        kind: crate::run::kind_kebab(node.kind),
        branch: &branch,
        prompt_file: &prompt_path,
        layout: None,
        no_hooks: false,
        keep_tmux_on_error: false,
        parent_session: manifest.managed_tmux_session.as_deref(),
        agent_startup_timeout: AGENT_RESPAWN_STARTUP_TIMEOUT,
        source_branch,
        cwd: source_repo,
    };
    let outcome = crate::run::spawn::run_create_sh_with_tmux_retry(&req)?;
    // Re-verify the freshly discovered PID is still alive (mirrors `run create`),
    // so a re-spawn that raced a just-died agent is treated as a spawn failure
    // rather than recording a dead pid onto the node.
    crate::run::spawn::verify_agent_pid(outcome.agent_pid_hint)?;
    Ok(RespawnOutcome {
        branch: outcome.branch,
        worktree_path: outcome.worktree_path,
        tmux_window: outcome.tmux_window,
        tmux_socket: outcome.tmux_socket,
        tmux_session: outcome.tmux_session,
        tmux_window_id: outcome.tmux_window_id,
        agent_pid: outcome.agent_pid_hint,
    })
}

/// Derive a FRESH branch name for a retry re-spawn from the dead worker's branch:
/// strip any prior `-rN` retry suffix, then append `-r<attempt>`. A distinct name
/// each attempt means the new worktree never collides with a stale one that
/// teardown could not remove. `None`/empty prior branch falls back to a generic
/// stem so create.sh still gets a valid branch.
fn retry_branch_name(prior: Option<&str>, attempt: u32) -> String {
    let stem = prior
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("wt/retry");
    let base = strip_retry_suffix(stem);
    format!("{base}-r{attempt}")
}

/// Strip a trailing `-r<digits>` retry suffix so successive retries do not
/// accumulate (`wt/foo-r1` → `wt/foo`, not `wt/foo-r1-r2`).
fn strip_retry_suffix(branch: &str) -> &str {
    if let Some(idx) = branch.rfind("-r") {
        let suffix = &branch[idx + 2..];
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            return &branch[..idx];
        }
    }
    branch
}

/// Synthesize the terminal `failed` `node.report` when a re-spawn's spawn
/// infrastructure is persistently broken (bounded by [`AGENT_RESPAWN_MAX_FAILURES`]).
///
/// Returns `true` when the node is durably terminal after this call — either this
/// call appended the failed report, OR it was already terminal (a `run cancel`
/// beat us). Returns `false` ONLY when the terminal outcome could NOT be recorded
/// (lock unavailable, or the append failed): the caller then KEEPS the park so
/// terminalization re-fires next tick, rather than un-tracking a still-non-terminal
/// node — which would let a supervisor restart re-park it at `spawn_failures = 0`
/// and re-enter the (broken) spawn loop (llm-review: bound-escape across restarts).
#[must_use]
fn terminalize_respawn_failure(
    paths: &RunPaths,
    nid: &NodeId,
    node_id: &str,
    attempt: u32,
    err: &str,
) -> bool {
    let guard = match RunLock::acquire(&paths.lock()) {
        Ok(g) => g,
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                node = %node_id, error = %e,
                "could not lock run to terminalize failed re-spawn; will retry next tick"
            );
            return false;
        }
    };
    let still_live = read_node_opt(paths, nid)
        .ok()
        .flatten()
        .is_some_and(|n| !matches!(n.status, Status::Done | Status::Failed | Status::Cancelled));
    if !still_live {
        // Already terminal (or unreadable → treated as settled): nothing to record,
        // and the park may be safely dropped.
        drop(guard);
        return true;
    }
    let data = json!({
        "success": false,
        "failed": true,
        "cancelled": false,
        "reason": "agent-respawn-failed",
        "summary": format!(
            "Node {node_id} died empty-handed; auto-retry re-spawn failed after {attempt} attempt(s): {err}"
        ),
        "discussion_items": [],
        "spinoff_proposals": [],
        "wrap_up_recommendations": [],
        "retry_attempts": attempt.saturating_sub(1),
    });
    let lock = guard.witness();
    let ok = match append_and_apply_unlocked(&lock, paths, "node.report", Some(nid), None, data) {
        Ok(_) => true,
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                node = %node_id, error = %e,
                "synthesize failed re-spawn report failed; will retry next tick"
            );
            false
        }
    };
    drop(guard);
    ok
}

fn watchdog_tick(
    paths: &RunPaths,
    half_state_streak: &mut std::collections::BTreeMap<String, u32>,
    retry_states: &mut std::collections::BTreeMap<String, RetryPark>,
    cpu_activity: &mut std::collections::BTreeMap<String, (u64, i64)>,
) -> Result<(), CliError> {
    let now = Utc::now();
    let now_instant = Instant::now();
    let grace = spawn_grace();
    // Scan our own nodes/ for any with an `agent_pid` that is running.
    // If a node is non-terminal and its agent has died (per dual-poll
    // protocol) AND it has not already produced a `node.report`,
    // synthesize one with `failed: true, reason: "agent-died"`.
    // Nodes that presented the `TmuxGone` half-state on THIS tick. Any
    // node not in this set (alive, dead, terminal, missing agent_pid,
    // unreadable) gets its streak dropped at end-of-tick, so the streak
    // counts strictly *consecutive* half-state ticks and never leaks.
    let mut tmux_gone_this_tick: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    // First pass: collect every non-terminal node that has a live `agent_pid`,
    // and union the distinct tmux sockets they probe. This lets the tick issue
    // ONE `tmux list-windows` per socket (`watchdog-batch-tmux-probe`) instead
    // of one subprocess per node — at ~100 agents that is 1 fork/tick, not 100.
    let mut candidates: Vec<(String, NodeId, Node, watchdog::AgentProbe)> = Vec::new();
    let mut sockets: std::collections::BTreeSet<Option<String>> = std::collections::BTreeSet::new();
    // This first pass is the supervisor's highest-frequency multi-file read, so
    // it is the one most likely to observe torn state. Hold the run's shared
    // lock for the whole `nodes/` scan so a concurrent reducer cannot mutate the
    // projection set under us (design.md §4); release it before the (slow) tmux
    // probing and the exclusive-locked report synthesis below. `probe_socket`
    // here only computes paths — the actual `tmux list-windows` runs after the
    // lock is dropped.
    RunLock::with_shared_lock(&paths.lock(), || {
        let entries = match std::fs::read_dir(paths.nodes_dir()) {
            Ok(v) => v,
            // No `nodes/` yet: nothing to scan, leave `candidates` empty.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(octl_core::Error::io(paths.nodes_dir(), e)),
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(node_id) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            let Ok(nid) = NodeId::parse_str(&node_id) else {
                continue;
            };
            let Ok(Some(n)) = read_node_opt(paths, &nid) else {
                continue;
            };
            if matches!(n.status, Status::Done | Status::Failed | Status::Cancelled) {
                continue;
            }
            // Retry-park gate (issue `autoretry-agent-died-worker`): a node parked
            // for bounded auto-retry after an empty-handed `agent-died` is owned by
            // `reconcile_agent_retries` until its backoff elapses and it is
            // re-spawned. It still carries the DEAD agent's pid, so leaving it in
            // the liveness scan would re-detect the same death every tick and
            // double-count the retry. Skip it here; the reconcile pass drives it.
            if retry_states.contains_key(&node_id) {
                continue;
            }
            // Spawn-grace gate (see `WATCHDOG_SPAWN_GRACE`): a node younger than
            // the grace window is skipped entirely — not probed, not streak-
            // tracked — so a fresh-spawn PID-discovery race can never synthesize a
            // terminal report that auto-cleanup would then act on. The PID was
            // verified alive at `node.created`, so the agent gets these seconds to
            // become visible before the watchdog is allowed to judge it dead.
            if within_spawn_grace(n.started_at, now, grace) {
                continue;
            }
            let Some(pid) = n.agent_pid else { continue };
            let probe = watchdog::AgentProbe {
                pid: pid as u32,
                start_time: n.agent_pid_start_time.map(|t| t.timestamp().max(0) as u64),
                tmux_window: n.tmux_window.clone(),
                tmux_identity: n.tmux_identity.clone(),
                // Skip the tmux probe only when there is neither a qualified
                // identity nor a legacy window name to probe with — don't fail
                // liveness on that absence alone. When present, the qualified
                // identity is preferred; the window name is the legacy fallback.
                skip_tmux_check: n.tmux_identity.is_none() && n.tmux_window.is_none(),
            };
            let (probes_tmux, socket) = probe.probe_socket();
            if probes_tmux {
                sockets.insert(socket);
            }
            candidates.push((node_id, nid, n, probe));
        }
        Ok(())
    })
    .map_err(from_core)?;

    // One timed `tmux list-windows -a` per distinct socket for the whole tick.
    // A wedged server is bounded by an internal timeout and yields an
    // "unreachable" socket (→ PID-only liveness), not a stalled tick
    // (`watchdog-tmux-probe-timeout`).
    let tmux_snapshot = watchdog::WatchdogTmuxSnapshot::collect(&sockets);

    // The git binary the reconcile probes shell out to (honors `GIT_BIN`).
    let git = cleanup::git_bin();

    // Node ids scanned this tick — used to prune the CPU-activity tracker below
    // so it cannot outlive the nodes it tracks.
    let scanned_ids: std::collections::BTreeSet<String> =
        candidates.iter().map(|(id, ..)| id.clone()).collect();

    for (node_id, nid, n, probe) in candidates {
        // Lifecycle-aware liveness. For AUTONOMOUS nodes the recorded agent_pid is
        // authoritative (a dead single fire-and-forget process is genuinely dead).
        // For INTERACTIVE nodes a human may quit and restart the agent (or it
        // re-execs on update/compaction) while the tmux window and their work live
        // on, so a bare PID probe fires a false `agent-died`; there the window is
        // the liveness signal (issue `agent-died-merge-no-teardown-interactive`).
        let interactive = n.kind.lifecycle() == Lifecycle::Interactive;
        let v = watchdog::check_liveness_for_lifecycle(&probe, &tmux_snapshot, interactive);
        // §7.5: commit `Dead`/`Recycled` immediately (PID gone or
        // recycled is unambiguous), but require a short retry streak for
        // the `TmuxGone` half-state so a transient `tmux list-windows`
        // hiccup does not kill a live agent.
        let commit = match v {
            watchdog::Liveness::Alive => false,
            watchdog::Liveness::Dead | watchdog::Liveness::Recycled => true,
            watchdog::Liveness::TmuxGone => {
                tmux_gone_this_tick.insert(node_id.clone());
                let c = half_state_streak.entry(node_id.clone()).or_insert(0);
                *c += 1;
                *c >= HALF_STATE_TICKS
            }
        };

        // Git-reconcile fallback (issues `false-failed-after-merge` /
        // `supervisor-stuck-pending-after-self-merge`). Before we ever classify a
        // node FAILED on liveness loss — and even while the agent is still ALIVE
        // but its terminal report never landed (the stuck-pending race) — ask git
        // whether the branch has already merged into the run's source (with a clean
        // worktree). If so, the work landed: the authoritative terminal outcome is
        // SUCCESS, not a false `failed`/`branch_preserved` and not an endless
        // `pending`. Gated PER NODE on the node's own lifecycle — autonomous kinds
        // self-merge fire-and-forget; interactive kinds keep human-owned windows.
        // This first probe is OUTSIDE the run lock (git is slow); it is RE-VERIFIED
        // under the lock before synthesis to close the probe→synthesis TOCTOU (a
        // live agent could move its branch in between). `node_branch_merged_to_source`
        // is conservative — it requires the branch to have advanced FORWARD past its
        // recorded fork point AND the worktree to be clean, so a fresh/rewound agent
        // or one with live uncommitted work is never reconciled or torn down.
        let reconcile_eligible =
            n.kind.lifecycle() == Lifecycle::Autonomous && n.last_report.is_none();
        let reconcile_probe =
            reconcile_eligible && cleanup::node_branch_merged_to_source(paths, &n, &git);

        if n.last_report.is_none() && (reconcile_probe || commit) {
            // The `last_report.is_none()` we tested (and the reconcile probe) ran
            // OUTSIDE the run lock, so a real `node.report` could land in the
            // window before this synthesis. Acquire the run flock and re-read the
            // node projection under it: only synthesize if the node is STILL
            // non-terminal and STILL unreported. This closes the F15
            // duplicate-terminal-report race (the parent-side reducer dedups it
            // anyway, but we avoid emitting the second event at all).
            // `append_and_apply_unlocked` is the sanctioned lock-held path —
            // re-entering `append_and_apply_event` here would deadlock on the flock.
            let guard = match RunLock::acquire(&paths.lock()) {
                Ok(g) => g,
                Err(e) => {
                    warn!(
                        target: "orchestratectl::supervise",
                        node = %node_id,
                        error = %e,
                        "watchdog could not lock run to synthesize report"
                    );
                    continue;
                }
            };
            let fresh = read_node_opt(paths, &nid).ok().flatten();
            let still_synthesizable = fresh.as_ref().is_some_and(|n| {
                n.last_report.is_none()
                    && !matches!(n.status, Status::Done | Status::Failed | Status::Cancelled)
            });
            if !still_synthesizable {
                tracing::debug!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    "watchdog deferred to live report"
                );
                drop(guard);
                continue;
            }
            // Re-verify the merge UNDER the lock against the fresh projection, so
            // the destructive `merge-reconciled` success reflects git state as of
            // the moment we commit it — not a stale probe from before an alive
            // agent moved its branch. If the branch is no longer merged (or the
            // worktree turned dirty), fall through to the liveness verdict.
            let reconciled = reconcile_probe
                && fresh
                    .as_ref()
                    .is_some_and(|f| cleanup::node_branch_merged_to_source(paths, f, &git));
            if !reconciled && !commit {
                // The probe said "merged" but the under-lock re-check disagrees and
                // the agent is still alive — nothing terminal to record this tick.
                tracing::debug!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    "reconcile no longer holds under lock; leaving live node alone"
                );
                drop(guard);
                continue;
            }
            // Bounded auto-retry park (issue `autoretry-agent-died-worker`). A
            // death that is NOT a reconciled-merge is a genuine `agent-died`.
            // Before terminalizing, decide whether this is a recoverable transient
            // death of an autonomous single-node worker that committed NOTHING —
            // if so, park it for a re-spawn instead of writing the failed report.
            // Gated on ALL of: the strong `Dead` verdict (the PID is provably gone
            // per the dual-poll protocol — NOT the weaker `Recycled` / `TmuxGone`
            // half-states, where the real agent could still be alive and committing,
            // making the destructive teardown-and-respawn unsafe), a retry-eligible
            // kind, POSITIVELY empty-handed (git-confirmed zero commits ahead of
            // source AND a clean worktree — the retry ⟂ salvage split), and attempts
            // remaining. An exhausted budget (or a weaker death verdict) falls
            // through to the failed report below (stamped with `retry_attempts`), so
            // a persistently-dying agent still terminalizes bounded.
            if !reconciled && matches!(v, watchdog::Liveness::Dead) {
                if let Some(f) = fresh.as_ref() {
                    if retry_eligible_kind(f) && cleanup::node_is_empty_handed(paths, f, &git) {
                        let attempts = f.retry_attempts;
                        let max = agent_retry_max_attempts();
                        if attempts < max {
                            let attempt = attempts + 1;
                            let backoff = agent_retry_backoff(attempt);
                            info!(
                                target: "orchestratectl::supervise",
                                node = %node_id,
                                attempt,
                                backoff_secs = backoff.as_secs(),
                                "empty-handed agent-died on autonomous worker; parking for bounded auto-retry"
                            );
                            retry_states.insert(
                                node_id.clone(),
                                RetryPark {
                                    attempt,
                                    retry_at: now_instant + backoff,
                                    reason: v.reason().to_string(),
                                    spawn_failures: 0,
                                },
                            );
                            drop(guard);
                            continue;
                        }
                        info!(
                            target: "orchestratectl::supervise",
                            node = %node_id,
                            attempts,
                            max,
                            "empty-handed agent-died but retry budget exhausted; terminalizing failed"
                        );
                    }
                }
            }
            // Synthesize a terminal node.report under the held run flock. A
            // git-confirmed merge is a terminal SUCCESS (stamped
            // `via: VIA_MERGE_RECONCILED` so cleanup force-tears-down exactly like
            // an explicit merge — never a false `branch_preserved`); otherwise the
            // liveness verdict stands and the node is `agent-died` FAILED.
            let data = if reconciled {
                info!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    "branch already merged into source; reconciling run to success (lost terminal report)"
                );
                json!({
                    "success": true,
                    "failed": false,
                    "cancelled": false,
                    "via": cleanup::VIA_MERGE_RECONCILED,
                    "reason": "branch-merged-to-source",
                    "summary": format!(
                        "Node {} branch already merged into source; supervisor reconciled run to success (agent's terminal report was lost).",
                        node_id
                    ),
                    "discussion_items": [],
                    "spinoff_proposals": [],
                    "wrap_up_recommendations": [],
                })
            } else {
                // The agent's process died before merging. Before recording the
                // bare failure, ask git whether it left salvageable work: commits
                // ahead of source that merge cleanly (issue
                // `agent-death-strands-recoverable-work`). A `Some` signal is only
                // produced when the branch carries unmerged commits, so a genuine
                // empty-handed death leaves the failed envelope byte-for-byte
                // unchanged. Computed against the SAME `fresh` projection the
                // under-lock re-verify used, so the signal reflects git state as of
                // the moment the report is committed.
                let recoverability = fresh
                    .as_ref()
                    .and_then(|f| cleanup::node_recoverability(paths, f, &git));
                if let Some(r) = &recoverability {
                    info!(
                        target: "orchestratectl::supervise",
                        node = %node_id,
                        branch = %r.branch,
                        unmerged_commits = r.unmerged_commits,
                        merges_cleanly = r.merges_cleanly,
                        "agent died leaving unmerged commits; stamping recoverability signal into failed report"
                    );
                }
                let mut data = json!({
                    "success": false,
                    "failed": true,
                    "cancelled": false,
                    "reason": v.reason(),
                    "summary": format!("Agent for node {} stopped responding: {}", node_id, v.reason()),
                    "discussion_items": [],
                    "spinoff_proposals": [],
                    "wrap_up_recommendations": [],
                });
                if let Some(r) = recoverability {
                    // `data` is the object literal above, so this always inserts.
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("recoverable_work".to_string(), r.to_report_value());
                    }
                }
                // If this failure is an EXHAUSTED bounded-retry (the node was
                // re-spawned `retry_attempts` times before finally dying
                // empty-handed), record the count on the report so the terminal
                // outcome is auditable — "failed after N auto-retries", not a bare
                // first-death failure. Absent (byte-for-byte unchanged envelope)
                // when the node was never retried.
                let retried = fresh.as_ref().map_or(0, |f| f.retry_attempts);
                if retried > 0 {
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("retry_attempts".to_string(), json!(retried));
                    }
                }
                data
            };
            // The `guard` above proves the exclusive lock is held; mint the
            // witness to thread into the unlocked append.
            let lock = guard.witness();
            if let Err(e) =
                append_and_apply_unlocked(&lock, paths, "node.report", Some(&nid), None, data)
            {
                warn!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    error = %e,
                    "synthesize node.report failed"
                );
            }
            drop(guard);
        }

        // ---- Idle-unmerged safety net (issue `agent-skips-run-merge-idle-pending`) ----
        // The reconcile path above rescues an ALREADY-MERGED branch whose report
        // was lost; the death path rescues a DEAD agent's stranded work. Neither
        // covers the third gap: an autonomous agent that committed cleanly-mergeable
        // work, left a clean worktree, then skipped its mandatory `run merge` and
        // dropped to an idle shell. Its PID stays Alive and no terminal report ever
        // lands, so `rollup_status` returns `None` forever and the run hangs
        // `pending` (a live supervisor + tmux window + worktree leaked per
        // occurrence). Detect it and terminalize to a RECOVERABLE failed state a
        // conductor can salvage with `run merge` — the same manual recovery the
        // issue describes, now bounded instead of infinite.
        //
        // Gated on ALL of: liveness ALIVE (a DEAD agent's committed work is already
        // salvaged by the death path above — never double-handle), NOT already
        // merged (the reconcile path owns that), no terminal report yet, and an
        // AUTONOMOUS lifecycle. Interactive `code`/`orchestrate` runs are EXEMPT — a
        // human owns their merge (`interactive-code-run-self-merged`); they keep
        // idling for `/worktree-merge`. The idle + clean-worktree + committed-work
        // checks (and the long-working-agent guard) live in
        // `cleanup::node_idle_unmerged`; the third (CPU) activity clock is sampled
        // here since it needs cross-tick state.
        if matches!(v, watchdog::Liveness::Alive)
            && !reconcile_probe
            && n.last_report.is_none()
            && n.kind.lifecycle() == Lifecycle::Autonomous
        {
            let threshold = idle_unmerged_threshold().as_secs() as i64;
            // Third activity clock: sample the agent's cumulative CPU time and
            // fold the last-change timestamp into the idle verdict, so an agent
            // that is busy but SILENT (no commits, no pane output) is not judged
            // idle. Sampled every tick this node reaches here so cross-tick deltas
            // are observed. A missing PID / unreadable CPU yields `None` → the
            // clock is simply omitted (commit + pane clocks still apply).
            let cpu_now = n.agent_pid.and_then(|p| {
                u32::try_from(p)
                    .ok()
                    .and_then(watchdog::pid_cpu_time_centis)
            });
            let cpu_clock = cpu_activity_clock(cpu_activity, &node_id, cpu_now, now.timestamp());
            // Cheap outside-lock probe (git only): gate the exclusive lock so we do
            // not contend with the reducer every tick for every live autonomous node.
            if cleanup::node_idle_unmerged(paths, &n, &git, now.timestamp(), threshold, cpu_clock)
                .is_some()
            {
                // The probe ran OUTSIDE the run lock. Acquire the flock, re-read the
                // node under it, and re-verify BOTH that it is still non-terminal &
                // unreported AND that the idle-unmerged verdict still holds against
                // the fresh projection + current git state — so a report that landed
                // in the window, or an agent that resumed and moved its branch /
                // dirtied its tree, aborts the synthesis. Mirrors the reconcile
                // path's TOCTOU close.
                let guard = match RunLock::acquire(&paths.lock()) {
                    Ok(g) => g,
                    Err(e) => {
                        warn!(
                            target: "orchestratectl::supervise",
                            node = %node_id,
                            error = %e,
                            "watchdog could not lock run to synthesize idle-unmerged report"
                        );
                        continue;
                    }
                };
                let fresh = read_node_opt(paths, &nid).ok().flatten();
                let still_unreported = fresh.as_ref().is_some_and(|f| {
                    f.last_report.is_none()
                        && !matches!(f.status, Status::Done | Status::Failed | Status::Cancelled)
                });
                // Reuse the CPU clock sampled above (do not re-sample: a second
                // read with the same `now` would be a redundant map update); the
                // git + fs clocks and the merge probe are re-run under the lock.
                let idle = fresh.as_ref().filter(|_| still_unreported).and_then(|f| {
                    cleanup::node_idle_unmerged(
                        paths,
                        f,
                        &git,
                        now.timestamp(),
                        threshold,
                        cpu_clock,
                    )
                });
                let Some(idle) = idle else {
                    tracing::debug!(
                        target: "orchestratectl::supervise",
                        node = %node_id,
                        "idle-unmerged no longer holds under lock; leaving live node alone"
                    );
                    drop(guard);
                    continue;
                };
                info!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    branch = %idle.recoverability.branch,
                    unmerged_commits = idle.recoverability.unmerged_commits,
                    merges_cleanly = idle.recoverability.merges_cleanly,
                    idle_secs = idle.idle_secs,
                    "autonomous agent committed but never merged and has gone idle; terminalizing run to recoverable failed"
                );
                // The salvage hint depends on whether the branch still merges
                // cleanly: a conflicting branch (main advanced under the idle
                // agent) is terminalized too, but needs a conflict resolution
                // first — say so instead of implying a plain `run merge` works.
                let salvage = if idle.recoverability.merges_cleanly {
                    "land it with `run merge`"
                } else {
                    "resolve conflicts against source, then `run merge` (see recoverable_work)"
                };
                let mut data = json!({
                    "success": false,
                    "failed": true,
                    "cancelled": false,
                    "reason": IDLE_UNMERGED_REASON,
                    "summary": format!(
                        "Agent for node {} committed work but never called `run merge` and has been idle {}s; supervisor terminalized the run recoverable ({}).",
                        node_id, idle.idle_secs, salvage
                    ),
                    "discussion_items": [],
                    "spinoff_proposals": [],
                    "wrap_up_recommendations": [],
                });
                if let Some(obj) = data.as_object_mut() {
                    obj.insert(
                        "recoverable_work".to_string(),
                        idle.recoverability.to_report_value(),
                    );
                }
                let lock = guard.witness();
                if let Err(e) =
                    append_and_apply_unlocked(&lock, paths, "node.report", Some(&nid), None, data)
                {
                    warn!(
                        target: "orchestratectl::supervise",
                        node = %node_id,
                        error = %e,
                        "synthesize idle-unmerged node.report failed"
                    );
                }
                drop(guard);
            }
        }
    }
    // Drive the bounded auto-retry state machine: for every parked node whose
    // backoff has elapsed, re-spawn a clean worker at the run's source branch (or
    // terminalize `failed` once the retry / spawn-failure budget is exhausted).
    // Runs AFTER the liveness scan so a fresh death parked THIS tick waits out its
    // backoff before the first re-spawn (issue `autoretry-agent-died-worker`).
    reconcile_agent_retries(paths, retry_states, now_instant);

    // Drop streaks for every node that did NOT present `TmuxGone` this
    // tick (committed, recovered, terminal, or no longer scanned) so the
    // count is strictly consecutive and the map cannot grow unbounded.
    half_state_streak.retain(|k, _| tmux_gone_this_tick.contains(k));
    // Prune the CPU-activity tracker to nodes still scanned this tick, so a
    // terminalized / removed node's entry cannot linger (bounded like the
    // streak map above).
    cpu_activity.retain(|k, _| scanned_ids.contains(k));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `maybe_warn_dropped` warns on the first new drop, then suppresses
    /// further warnings inside `DROPPED_WARN_INTERVAL`, then warns again once
    /// the interval has elapsed AND the count has grown further. A static
    /// count never re-warns. Clock + count are injected so the rate-limit is
    /// asserted deterministically without real time or a live appender.
    #[test]
    fn maybe_warn_dropped_rate_limits_on_increase() {
        let t0 = Instant::now();
        let mut last_count = 0u64;
        let mut last_at: Option<Instant> = None;

        // First observed drops → warns and records the count + timestamp.
        assert!(maybe_warn_dropped(5, t0, &mut last_count, &mut last_at));
        assert_eq!(last_count, 5);
        assert_eq!(last_at, Some(t0));

        // More drops, but well inside the interval → suppressed, state frozen.
        assert!(!maybe_warn_dropped(
            9,
            t0 + Duration::from_secs(30),
            &mut last_count,
            &mut last_at
        ));
        assert_eq!(last_count, 5);
        assert_eq!(last_at, Some(t0));

        // Interval elapsed and the count grew further → warns again.
        let t1 = t0 + DROPPED_WARN_INTERVAL + Duration::from_secs(1);
        assert!(maybe_warn_dropped(9, t1, &mut last_count, &mut last_at));
        assert_eq!(last_count, 9);
        assert_eq!(last_at, Some(t1));

        // No new drops since the last warning → never warns, even much later.
        assert!(!maybe_warn_dropped(
            9,
            t1 + Duration::from_secs(600),
            &mut last_count,
            &mut last_at
        ));
        assert_eq!(last_count, 9);
    }

    /// The spawn-grace predicate: a node younger than `grace` is in-grace; at
    /// or past `grace` it is eligible; a missing `started_at` is eligible (we
    /// cannot prove it fresh); and a backwards clock is conservatively
    /// in-grace. All timestamps are injected so the comparison is exact and
    /// does not depend on wall-clock timing.
    #[test]
    fn within_spawn_grace_boundaries() {
        let grace = Duration::from_secs(5);
        let created = "2026-06-28T12:00:00Z".parse::<DateTime<Utc>>().unwrap();

        // Fresh: 1s old, well inside the 5s window → in-grace.
        let now = created + chrono::Duration::seconds(1);
        assert!(within_spawn_grace(Some(created), now, grace));

        // Just under the boundary (4.999s) → still in-grace.
        let now = created + chrono::Duration::milliseconds(4_999);
        assert!(within_spawn_grace(Some(created), now, grace));

        // Exactly at the boundary (5s) → eligible (age < grace is strict).
        let now = created + chrono::Duration::seconds(5);
        assert!(!within_spawn_grace(Some(created), now, grace));

        // Well past the window → eligible.
        let now = created + chrono::Duration::seconds(60);
        assert!(!within_spawn_grace(Some(created), now, grace));

        // No started_at → eligible (cannot prove freshness).
        let now = created + chrono::Duration::seconds(1);
        assert!(!within_spawn_grace(None, now, grace));

        // Clock ran backwards (now before creation) → conservatively in-grace.
        let now = created - chrono::Duration::seconds(1);
        assert!(within_spawn_grace(Some(created), now, grace));

        // A zero grace disables suppression for any known-age node.
        let now = created + chrono::Duration::milliseconds(1);
        assert!(!within_spawn_grace(Some(created), now, Duration::ZERO));
    }

    /// The child-supervisor startup state machine
    /// (issue `child-supervisor-spawn-unconfirmed-no-retry`). Injected clock so
    /// every transition is asserted deterministically. The load-bearing
    /// invariant: an absent pid NEVER yields `Confirm` — a child that never
    /// wrote a pid is held or retried, never recorded as started at pid 0.
    #[test]
    fn child_spawn_action_state_machine() {
        let t0 = Instant::now();
        let deadline = Duration::from_secs(10);
        let max = 3;

        let starting = ChildSpawn::Starting {
            since: t0,
            attempts: 1,
        };

        // Starting, no pid, inside the deadline → Wait (NOT confirmed at 0).
        assert_eq!(
            child_spawn_action(&starting, None, t0 + Duration::from_secs(1), deadline, max),
            SpawnAction::Wait
        );
        // Starting, no pid, deadline reached → MarkFailed.
        assert_eq!(
            child_spawn_action(&starting, None, t0 + deadline, deadline, max),
            SpawnAction::MarkFailed
        );
        // Starting, an identity-verified pid appears → Confirm (that pid).
        assert_eq!(
            child_spawn_action(
                &starting,
                Some(4321),
                t0 + Duration::from_secs(1),
                deadline,
                max
            ),
            SpawnAction::Confirm(4321)
        );

        let failed = ChildSpawn::Failed {
            attempts: 1,
            retry_at: t0 + Duration::from_secs(5),
        };
        // Failed, before retry_at → Wait.
        assert_eq!(
            child_spawn_action(&failed, None, t0 + Duration::from_secs(1), deadline, max),
            SpawnAction::Wait
        );
        // Failed, retry_at reached, attempts remain → Retry.
        assert_eq!(
            child_spawn_action(&failed, None, t0 + Duration::from_secs(5), deadline, max),
            SpawnAction::Retry
        );
        // Failed, a late pid finally shows up → adopt it (Confirm), don't re-fork.
        assert_eq!(
            child_spawn_action(
                &failed,
                Some(99),
                t0 + Duration::from_secs(5),
                deadline,
                max
            ),
            SpawnAction::Confirm(99)
        );

        // Retry budget exhausted → Wait (bounded, never an unbounded loop).
        let exhausted = ChildSpawn::Failed {
            attempts: max,
            retry_at: t0,
        };
        assert_eq!(
            child_spawn_action(
                &exhausted,
                None,
                t0 + Duration::from_secs(100),
                deadline,
                max
            ),
            SpawnAction::Wait
        );
        // …but even an exhausted child adopts a pid that finally appears.
        assert_eq!(
            child_spawn_action(
                &exhausted,
                Some(7),
                t0 + Duration::from_secs(100),
                deadline,
                max
            ),
            SpawnAction::Confirm(7)
        );
    }

    /// Backoff is bounded and non-decreasing, and never overflows for a large
    /// attempt count.
    #[test]
    fn child_retry_backoff_is_bounded() {
        assert_eq!(child_retry_backoff(1), CHILD_RETRY_BASE_BACKOFF);
        assert!(child_retry_backoff(2) >= child_retry_backoff(1));
        assert!(child_retry_backoff(3) >= child_retry_backoff(2));
        assert_eq!(child_retry_backoff(100), CHILD_RETRY_MAX_BACKOFF);
        // Every value stays within the ceiling.
        for a in 1..50 {
            assert!(child_retry_backoff(a) <= CHILD_RETRY_MAX_BACKOFF);
        }
    }

    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn agent_retry_backoff_is_bounded_and_monotone() {
        // Serialized against the retry integration tests (which set
        // `OCTL_AGENT_RETRY_BACKOFF_SECS`) and defensively cleared, so this reads
        // the compiled default rather than a value another test left set.
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let prior = std::env::var_os(AGENT_RETRY_BACKOFF_ENV);
        std::env::remove_var(AGENT_RETRY_BACKOFF_ENV);
        assert_eq!(agent_retry_backoff(1), AGENT_RETRY_BASE_BACKOFF);
        assert!(agent_retry_backoff(2) >= agent_retry_backoff(1));
        assert_eq!(agent_retry_backoff(100), AGENT_RETRY_MAX_BACKOFF);
        for a in 1..50 {
            assert!(agent_retry_backoff(a) <= AGENT_RETRY_MAX_BACKOFF);
        }
        // A huge env override must clamp, not panic on overflow.
        std::env::set_var(AGENT_RETRY_BACKOFF_ENV, u64::MAX.to_string());
        assert_eq!(agent_retry_backoff(6), AGENT_RETRY_MAX_BACKOFF);
        match prior {
            Some(v) => std::env::set_var(AGENT_RETRY_BACKOFF_ENV, v),
            None => std::env::remove_var(AGENT_RETRY_BACKOFF_ENV),
        }
    }

    #[test]
    fn retry_branch_name_appends_and_does_not_accumulate_suffix() {
        assert_eq!(retry_branch_name(Some("wt/foo"), 1), "wt/foo-r1");
        // A prior retry suffix is stripped, not stacked.
        assert_eq!(retry_branch_name(Some("wt/foo-r1"), 2), "wt/foo-r2");
        assert_eq!(retry_branch_name(Some("wt/foo-r2"), 3), "wt/foo-r3");
        // A `-r` that is NOT a retry suffix (non-numeric) is preserved.
        assert_eq!(retry_branch_name(Some("wt/re-run"), 1), "wt/re-run-r1");
        // Empty / missing prior branch falls back to a valid stem.
        assert_eq!(retry_branch_name(None, 1), "wt/retry-r1");
        assert_eq!(retry_branch_name(Some(""), 2), "wt/retry-r2");
    }

    /// A bare non-terminal node of `kind` with no parent — enough to exercise the
    /// pure eligibility gate.
    fn minimal_node(kind: octl_core::Kind) -> Node {
        Node {
            schema_version: 1,
            node_id: NodeId::parse_str("n-0001").unwrap(),
            run_id: octl_core::RunId::parse_str("01jxwd0000000000000000000w").unwrap(),
            parent_node_id: None,
            kind,
            status: Status::Running,
            task: None,
            worktree_path: Some("/tmp/wt".to_string()),
            branch: Some("wt/foo".to_string()),
            base_sha: None,
            tmux_window: None,
            tmux_identity: None,
            agent_pid: Some(4242),
            agent_pid_start_time: None,
            supervisor_pid: None,
            children: Vec::new(),
            started_at: None,
            updated_at: Utc::now(),
            last_report: None,
            last_processed_report_seq_by_child: serde_json::Map::default(),
            retry_attempts: 0,
        }
    }

    #[test]
    fn retry_eligible_kind_matches_autonomous_single_node_workers() {
        use octl_core::Kind;
        let mut n = minimal_node(Kind::Spinoff);
        assert!(
            retry_eligible_kind(&n),
            "top-level autonomous spinoff is eligible"
        );
        // A child (has a parent) is never independently retried.
        n.parent_node_id = Some(NodeId::parse_str("n-0002").unwrap());
        assert!(!retry_eligible_kind(&n), "a DAG child is not eligible");
        // Interactive kind is never force-retried.
        let code = minimal_node(Kind::Code);
        assert!(!retry_eligible_kind(&code));
    }

    /// End-to-end reconcile of the failure path: a child forked long ago whose
    /// identity-verified pid never appeared (no pid file on disk) must NOT be
    /// recorded into `spawned_children` (the pid-0-as-success bug) and MUST be
    /// scheduled for a bounded retry instead. Exercises the real
    /// `reconcile_child_spawns` (including `read_live_recorded_pid` returning
    /// `None`); no process is forked because this hits `MarkFailed`, not
    /// `Retry`.
    #[test]
    fn reconcile_never_records_unconfirmed_child_and_schedules_retry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        // A real parent run dir so the `child.spawn_failed` append lands.
        let parent_id = "01jxsnap000000000000000000";
        let parent_dir = root.join(parent_id);
        std::fs::create_dir_all(&parent_dir).unwrap();
        let parent_paths = RunPaths::new(parent_dir, parent_id).unwrap();
        append_and_apply_event(
            &parent_paths,
            "run.created",
            None,
            None,
            json!({ "kind": "orchestrate", "lifecycle": "autonomous", "title": "drive" }),
        )
        .unwrap();

        // A child that was forked but never wrote a pid file: its run dir does
        // not even exist, so `read_live_recorded_pid` returns `None`.
        let child_id = "01jxsnap000000000000000042".to_string();
        let base = Instant::now();
        let mut child_spawns = std::collections::BTreeMap::new();
        child_spawns.insert(
            child_id.clone(),
            ChildSpawn::Starting {
                since: base,
                attempts: 1,
            },
        );
        let mut state = state::SupervisorState::default();

        // Tick with a clock well past the deadline.
        let now = base + CHILD_SPAWN_DEADLINE + Duration::from_secs(1);
        reconcile_child_spawns(&root, &parent_paths, &mut child_spawns, &mut state, now);

        // THE bug guard: an unconfirmed child is never recorded as started.
        assert!(
            state.spawned_children.is_empty(),
            "unconfirmed child (pid 0) must never enter spawned_children"
        );
        // It is scheduled for a bounded retry, not dropped.
        assert!(
            matches!(
                child_spawns.get(&child_id),
                Some(ChildSpawn::Failed { attempts: 1, .. })
            ),
            "expected Failed{{attempts:1}}, got {:?}",
            child_spawns.get(&child_id)
        );

        // A `child.spawn_failed` audit record landed on the parent log.
        let raw = std::fs::read_to_string(parent_paths.events()).unwrap();
        assert!(
            raw.contains("child.spawn_failed"),
            "reconcile must record child.spawn_failed on the parent log"
        );
    }

    /// Restart recovery: a forked-but-unconfirmed child must be re-seeded as
    /// `Starting` on boot so the reconcile pass adopts or re-forks it — the
    /// never-retried bug otherwise returns on the parent-restart path.
    /// Confirmed-running children and terminal children are left alone.
    #[test]
    fn reseed_child_spawns_recovers_unconfirmed_children_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let now = Instant::now();

        // Create a child run dir at the canonical `root/runs/<id>` layout that
        // `run_paths` (and therefore `reseed_child_spawns`) resolves to, and
        // return its `RunPaths` for appends.
        let make_child = |id: &str| -> RunPaths {
            let paths = run_paths(root, id).unwrap();
            std::fs::create_dir_all(octl_core::run_dir(root, &parse_run_id(id).unwrap())).unwrap();
            append_and_apply_event(
                &paths,
                "run.created",
                None,
                None,
                json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": id }),
            )
            .unwrap();
            paths
        };

        // A non-terminal child that was never confirmed → must be re-seeded.
        let pending = "01jxsnap000000000000000001";
        make_child(pending);

        // A terminal child → needs no supervisor, must be skipped.
        let done = "01jxsnap000000000000000002";
        let done_paths = make_child(done);
        append_and_apply_event(
            &done_paths,
            "run.status",
            None,
            None,
            json!({ "status": "done" }),
        )
        .unwrap();

        // A child already recorded as confirmed-running → must be skipped.
        let confirmed = "01jxsnap000000000000000003".to_string();
        let mut state = state::SupervisorState::default();
        state.spawned_children.insert(confirmed.clone(), 4242);

        let ids = [pending, done, confirmed.as_str()];
        let seeded = reseed_child_spawns(root, ids.iter().copied(), &state, now);

        assert!(
            matches!(
                seeded.get(pending),
                Some(ChildSpawn::Starting { attempts: 1, .. })
            ),
            "an unconfirmed non-terminal child must be re-seeded as Starting"
        );
        assert!(
            !seeded.contains_key(done),
            "a terminal child needs no supervisor and must not be re-seeded"
        );
        assert!(
            !seeded.contains_key(&confirmed),
            "a confirmed-running child must not be re-seeded (would double-track)"
        );
    }

    /// The exhaustion path (`MarkFailed` when the budget is spent) records
    /// `"final": true` on the `child.spawn_failed` event, and a mid-retry
    /// failure records `"final": false` — so operators can tell "giving up" from
    /// "will retry" (the log wording used to always say "scheduling retry").
    #[test]
    fn reconcile_marks_final_failure_on_budget_exhaustion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        let parent_id = "01jxsnap000000000000000000";
        let parent_dir = root.join(parent_id);
        std::fs::create_dir_all(&parent_dir).unwrap();
        let parent_paths = RunPaths::new(parent_dir, parent_id).unwrap();
        append_and_apply_event(
            &parent_paths,
            "run.created",
            None,
            None,
            json!({ "kind": "orchestrate", "lifecycle": "autonomous", "title": "drive" }),
        )
        .unwrap();

        let child_id = "01jxsnap000000000000000099".to_string();
        let base = Instant::now();
        let now = base + CHILD_SPAWN_DEADLINE + Duration::from_secs(1);

        // Last attempt in flight: `Starting{attempts=max}`; deadline passed, no pid.
        let mut child_spawns = std::collections::BTreeMap::new();
        child_spawns.insert(
            child_id.clone(),
            ChildSpawn::Starting {
                since: base,
                attempts: CHILD_SPAWN_MAX_ATTEMPTS,
            },
        );
        let mut state = state::SupervisorState::default();
        reconcile_child_spawns(&root, &parent_paths, &mut child_spawns, &mut state, now);

        // Never confirmed, and the last failure is flagged final.
        assert!(state.spawned_children.is_empty());
        let raw = std::fs::read_to_string(parent_paths.events()).unwrap();
        assert!(
            raw.contains("\"final\":true"),
            "the exhausting failure must be recorded as final; log was: {raw}"
        );
    }

    // --- Watchdog git-reconcile fallback (issues `false-failed-after-merge` /
    // `supervisor-stuck-pending-after-self-merge`) --------------------------------
    //
    // These drive `watchdog_tick` directly (no real `supervise` subprocess, so no
    // `#[file_serial]` is required) against a real git repo whose spinoff branch
    // has already self-merged into `main`, with the terminal `node.report` never
    // emitted. They serialize on the process-global `OCTL_WATCHDOG_GRACE_SECS`
    // they set (grace 0, so a just-created node is eligible immediately).

    use std::process::{Command as PCommand, Stdio};
    use std::sync::Mutex;

    static GRACE_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII env guard: restores the prior value / unsets on drop so a panicking
    /// assertion cannot leak `OCTL_WATCHDOG_GRACE_SECS` into another test.
    struct GraceGuard(Option<std::ffi::OsString>);
    impl GraceGuard {
        fn zero() -> Self {
            let old = std::env::var_os(SPAWN_GRACE_ENV);
            std::env::set_var(SPAWN_GRACE_ENV, "0");
            Self(old)
        }
    }
    impl Drop for GraceGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var(SPAWN_GRACE_ENV, v),
                None => std::env::remove_var(SPAWN_GRACE_ENV),
            }
        }
    }

    fn tgit(cwd: &Path, args: &[&str]) {
        let ok = PCommand::new("git")
            .current_dir(cwd)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed in {cwd:?}");
    }

    fn trev(repo: &Path, r: &str) -> String {
        let out = PCommand::new("git")
            .current_dir(repo)
            .args(["rev-parse", r])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn tbranch_exists(repo: &Path, branch: &str) -> bool {
        PCommand::new("git")
            .current_dir(repo)
            .args(["rev-parse", "--verify", "--quiet", branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// Init a repo on `main`, fork `wt/foo`, commit work in it, and fast-forward
    /// `main` up to it — i.e. the spinoff's self-merge has landed. Returns
    /// `(repo, worktree, base_sha)` where `base_sha` is the fork point.
    fn init_merged_repo(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf, String) {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        tgit(&repo, &["init", "-q", "-b", "main"]);
        tgit(&repo, &["config", "user.email", "t@example.com"]);
        tgit(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        tgit(&repo, &["add", "-A"]);
        tgit(&repo, &["commit", "-qm", "init"]);
        let base = trev(&repo, "main");
        let wt = tmp.path().join("wt");
        tgit(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt/foo",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("fix.rs"), "work").unwrap();
        tgit(&wt, &["add", "-A"]);
        tgit(&wt, &["commit", "-qm", "agent work"]);
        tgit(&repo, &["merge", "--ff-only", "wt/foo"]); // the self-merge lands
        (repo, wt, base)
    }

    /// A run dir + autonomous-spinoff manifest + one worker node pointing at the
    /// merged worktree, with `agent_pid` (and no recorded `start_time`) set so
    /// the watchdog's liveness probe reads a real verdict.
    fn setup_merged_run(
        tmp: &tempfile::TempDir,
        repo: &Path,
        wt: &Path,
        base: &str,
        agent_pid: i32,
    ) -> RunPaths {
        let run_id = "01jxwd0000000000000000000w";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_repo": repo.to_str().unwrap(),
                "source_branch": "main",
            }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.created",
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            json!({
                "kind": "spinoff",
                "branch": "wt/foo",
                "base_sha": base,
                "worktree_path": wt.to_str().unwrap(),
                "agent_pid": agent_pid,
                // No tmux fields → the watchdog skips the tmux probe and liveness
                // is decided purely by the PID (Alive vs Dead).
            }),
        )
        .unwrap();
        paths
    }

    fn n0001(paths: &RunPaths) -> Node {
        read_node_opt(paths, &NodeId::parse_str("n-0001").unwrap())
            .unwrap()
            .unwrap()
    }

    /// Done-criterion (b): a merged branch whose agent has DIED (liveness loss)
    /// must reconcile to terminal SUCCESS — never the false `agent-died`/`failed`
    /// classification — and then tear down cleanly (worktree + branch gone, no
    /// `cleanup.branch_preserved`).
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_reconciles_merged_branch_on_agent_death() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_merged_repo(&tmp);
        // A definitely-dead pid: spawn `true`, reap it, reuse its pid.
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_merged_run(&tmp, &repo, &wt, &base, dead_pid);

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        let report = n
            .last_report
            .expect("a terminal report must be synthesized");
        assert_eq!(
            report["success"], true,
            "a merged branch reconciles to SUCCESS, not agent-died failure"
        );
        assert_eq!(report["via"], "merge-reconciled");
        assert_ne!(
            report["reason"], "agent-died",
            "must not be a false failure"
        );
        assert!(n.status.is_terminal());

        // The main loop's teardown chain: roll the run up and clean up.
        if let Some(status) = cleanup::rollup_status(&paths, true) {
            let s = if status == Status::Done {
                "done"
            } else {
                "failed"
            };
            append_and_apply_event(&paths, "run.status", None, None, json!({ "status": s }))
                .unwrap();
        }
        cleanup::cleanup_terminal_nodes(&paths);

        assert_eq!(
            read_manifest_opt(&paths).unwrap().unwrap().status,
            Status::Done,
            "reconciled run rolls up to Done"
        );
        assert!(!wt.exists(), "merged worktree is torn down");
        assert!(!tbranch_exists(&repo, "wt/foo"), "merged branch is deleted");
        let preserved = std::fs::read_to_string(paths.events())
            .unwrap()
            .lines()
            .any(|l| l.contains("cleanup.branch_preserved"));
        assert!(
            !preserved,
            "a merged branch must NEVER be reported preserved"
        );
    }

    /// Done-criterion (a): a merged branch whose agent is still ALIVE but whose
    /// terminal report never arrived (the stuck-pending race) is reconciled to
    /// terminal SUCCESS via the git fallback — the run cannot strand at `pending`
    /// forever waiting on a report that will never come.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_reconciles_merged_branch_while_agent_alive() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_merged_repo(&tmp);
        // A genuinely live agent process; reaped at end of test.
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_merged_run(&tmp, &repo, &wt, &base, alive_pid);

        // Sanity: the agent really is alive, so this is the stuck-pending path,
        // not the death path.
        assert!(pid_file::pid_alive(alive_pid as u32), "agent must be alive");

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        let report = n
            .last_report
            .expect("alive-but-merged must still reconcile to a terminal report");
        assert_eq!(report["success"], true);
        assert_eq!(report["via"], "merge-reconciled");
        assert!(n.status.is_terminal(), "run no longer strands at pending");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// The data-loss guard at the supervisor level (review finding #1): a LIVE
    /// agent whose branch merged but whose worktree still holds uncommitted work
    /// must NOT be reconciled — the watchdog synthesizes no report and leaves the
    /// node pending, so its live work is never torn down. This is the case the
    /// branch-ref-only check would have destroyed.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_leaves_live_agent_with_dirty_worktree_alone() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_merged_repo(&tmp);
        // The agent merged, then kept editing — uncommitted work in the worktree.
        std::fs::write(wt.join("still-working.rs"), "wip").unwrap();
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_merged_run(&tmp, &repo, &wt, &base, alive_pid);

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        assert!(
            n.last_report.is_none(),
            "a live agent with uncommitted work must NOT be reconciled/terminalized"
        );
        assert!(!n.status.is_terminal(), "node stays live");
        assert!(wt.exists(), "the worktree with live work is untouched");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    // --- Idle-unmerged safety net (issue `agent-skips-run-merge-idle-pending`) ---
    //
    // The complement of the reconcile tests above: the branch is committed and
    // cleanly mergeable but NOT merged, the agent is ALIVE (idle shell), and no
    // terminal report ever landed. The safety net must terminalize an AUTONOMOUS
    // run to a recoverable failed state, EXEMPT an interactive one, and NOT trip a
    // still-working agent. They serialize on `octl_watchdog_grace` (grace 0) and
    // set the process-global `OCTL_IDLE_UNMERGED_SECS`.

    /// Init a repo on `main`, fork `wt/foo`, commit work in it, but DO NOT merge
    /// into `main` — the committed-but-unmerged state. `commit_date`, when given,
    /// backdates the commit (an `RFC3339`/`git`-parseable date) so its tip time is
    /// stale; `None` commits at wall-clock now. Returns `(repo, worktree, base)`.
    fn init_unmerged_repo_at(
        tmp: &tempfile::TempDir,
        commit_date: Option<&str>,
    ) -> (PathBuf, PathBuf, String) {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        tgit(&repo, &["init", "-q", "-b", "main"]);
        tgit(&repo, &["config", "user.email", "t@example.com"]);
        tgit(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        tgit(&repo, &["add", "-A"]);
        tgit(&repo, &["commit", "-qm", "init"]);
        let base = trev(&repo, "main");
        let wt = tmp.path().join("wt");
        tgit(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt/foo",
                wt.to_str().unwrap(),
            ],
        );
        std::fs::write(wt.join("fix.rs"), "work").unwrap();
        tgit(&wt, &["add", "-A"]);
        let mut commit = PCommand::new("git");
        commit
            .current_dir(&wt)
            .args(["commit", "-qm", "agent work"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(d) = commit_date {
            commit
                .env("GIT_AUTHOR_DATE", d)
                .env("GIT_COMMITTER_DATE", d);
        }
        assert!(commit.status().unwrap().success(), "commit failed");
        // NB: no `git merge` — the work is left UNMERGED on the branch.
        (repo, wt, base)
    }

    fn init_unmerged_repo(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf, String) {
        init_unmerged_repo_at(tmp, None)
    }

    /// Like `init_unmerged_repo`, but ALSO advances `main` with a commit that
    /// adds the SAME file (`fix.rs`) with different content — so `wt/foo` and
    /// `main` both introduce `fix.rs` divergently (an add/add conflict) and the
    /// branch no longer merges cleanly into source.
    fn init_conflicting_unmerged_repo(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf, String) {
        let (repo, wt, base) = init_unmerged_repo(tmp);
        std::fs::write(repo.join("fix.rs"), "different").unwrap();
        tgit(&repo, &["add", "-A"]);
        tgit(&repo, &["commit", "-qm", "conflicting main change"]);
        (repo, wt, base)
    }

    /// A run dir + manifest (of `kind`/`lifecycle`) + one worker node pointing at
    /// the unmerged worktree, with a live `agent_pid`.
    fn setup_unmerged_run(
        tmp: &tempfile::TempDir,
        repo: &Path,
        wt: &Path,
        base: &str,
        agent_pid: i32,
        kind: &str,
        lifecycle: &str,
    ) -> RunPaths {
        let run_id = "01jxwd0000000000000000000w";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": kind,
                "lifecycle": lifecycle,
                "title": "t",
                "source_repo": repo.to_str().unwrap(),
                "source_branch": "main",
            }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.created",
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            json!({
                "kind": kind,
                "branch": "wt/foo",
                "base_sha": base,
                "worktree_path": wt.to_str().unwrap(),
                "agent_pid": agent_pid,
            }),
        )
        .unwrap();
        paths
    }

    /// Done-criterion: an AUTONOMOUS agent that committed cleanly-mergeable work,
    /// left a clean worktree, but never merged and has gone idle must be
    /// terminalized to a RECOVERABLE failed state (distinct `agent-idle-unmerged`
    /// reason + `recoverable_work`) — never left `pending` forever.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_terminalizes_idle_unmerged_autonomous_as_recoverable() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        // Fire on the first quiet tick (any idle ≥ 0s counts).
        let _idle = EnvGuard::set(IDLE_UNMERGED_ENV, "0");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_unmerged_repo(&tmp);
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, alive_pid, "spinoff", "autonomous");
        assert!(pid_file::pid_alive(alive_pid as u32), "agent must be alive");

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        let report = n
            .last_report
            .expect("idle-unmerged autonomous run must be terminalized, not left pending");
        assert_eq!(
            report["success"], false,
            "complete-but-unlanded is a failure"
        );
        assert_eq!(
            report["reason"], "agent-idle-unmerged",
            "reason distinguishes this from agent-died and blocked handoffs"
        );
        // The stranded work is surfaced so a conductor can `run merge` it.
        let rec = &report["recoverable_work"];
        assert_eq!(rec["recoverable"], true);
        assert_eq!(rec["unmerged_commits"], 1);
        assert_eq!(rec["merges_cleanly"], true);
        assert_eq!(rec["branch"], "wt/foo");
        assert!(n.status.is_terminal(), "run no longer strands at pending");
        // No `via` marker → the cleanup gate PRESERVES the branch/worktree for
        // salvage (invariant #5), rather than force-deleting it.
        assert!(report.get("via").is_none(), "recoverable, not a merge");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// An INTERACTIVE (`code`) run is EXEMPT: even committed + idle + unmerged, the
    /// human owns the merge (`interactive-code-run-self-merged`). The safety net
    /// must synthesize NO report and leave the node live for `/worktree-merge`.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_exempts_interactive_idle_unmerged() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _idle = EnvGuard::set(IDLE_UNMERGED_ENV, "0");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_unmerged_repo(&tmp);
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, alive_pid, "code", "interactive");

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        assert!(
            n.last_report.is_none(),
            "an interactive run must keep idling for the human merge, not be terminalized"
        );
        assert!(!n.status.is_terminal(), "node stays live");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// A legitimately long-running agent must NOT be tripped. Here the branch tip
    /// is FRESH (just committed) and the threshold is large: `now - last_activity`
    /// is far below it, so no report is synthesized. Guards the "heavy-LLM units
    /// run 54–96 min — do not terminalize those" requirement.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_does_not_trip_long_working_agent_with_fresh_commit() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        // A generous threshold; the fresh commit is well within it.
        let _idle = EnvGuard::set(IDLE_UNMERGED_ENV, "3600");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_unmerged_repo(&tmp);
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, alive_pid, "spinoff", "autonomous");

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        assert!(
            n.last_report.is_none(),
            "a still-working agent (fresh commit) must NOT be terminalized"
        );
        assert!(!n.status.is_terminal(), "node stays live");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// The activity signal takes the MAX of the branch-tip commit time and the
    /// pane-transcript (`agent.log`) mtime. A stale commit alone would trip the
    /// net, but a FRESH `agent.log` (the agent is still emitting pane output — a
    /// long `/llm-review` between commits) must hold it off. Directly exercises the
    /// commit-vs-pane composition that keeps a working-but-not-committing agent safe.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_does_not_trip_when_pane_active_despite_stale_commit() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _idle = EnvGuard::set(IDLE_UNMERGED_ENV, "3600");
        let tmp = tempfile::TempDir::new().unwrap();
        // Commit is ancient → its tip time alone would read as long-idle.
        let (repo, wt, base) = init_unmerged_repo_at(&tmp, Some("2020-01-01T00:00:00 +0000"));
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, alive_pid, "spinoff", "autonomous");
        // But the pane is still active NOW: a fresh agent.log mtime.
        std::fs::write(paths.agent_log(), b"...streaming model output...").unwrap();

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        assert!(
            n.last_report.is_none(),
            "a fresh pane transcript must hold off the net despite a stale commit"
        );
        assert!(!n.status.is_terminal(), "node stays live");

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// Regression for the `/llm-review` finding "conflicting branch still hangs":
    /// a committed-but-CONFLICTING idle branch (main advanced under the idle
    /// agent) must STILL be terminalized — with `recoverable: false` /
    /// `merges_cleanly: false` surfaced — so the run cannot hang `pending`
    /// forever merely because of a conflict. The blocked-report path still
    /// preserves the branch/worktree for manual conflict resolution.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn watchdog_terminalizes_idle_unmerged_conflicting_as_recoverable_false() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _idle = EnvGuard::set(IDLE_UNMERGED_ENV, "0");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_conflicting_unmerged_repo(&tmp);
        let mut alive = PCommand::new("sleep").arg("30").spawn().unwrap();
        let alive_pid = alive.id() as i32;
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, alive_pid, "spinoff", "autonomous");

        let mut streak = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut std::collections::BTreeMap::new(),
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        let report = n
            .last_report
            .expect("a conflicting idle-unmerged run must still be terminalized, not left pending");
        assert_eq!(report["success"], false);
        assert_eq!(report["reason"], "agent-idle-unmerged");
        let rec = &report["recoverable_work"];
        assert_eq!(
            rec["recoverable"], false,
            "a conflicting branch is not cleanly recoverable"
        );
        assert_eq!(rec["merges_cleanly"], false);
        assert_eq!(rec["unmerged_commits"], 1);
        assert!(n.status.is_terminal(), "run no longer strands at pending");
        assert!(
            report.get("via").is_none(),
            "recoverable, not a merge → cleanup preserves the branch for resolution"
        );

        let _ = alive.kill();
        let _ = alive.wait();
    }

    /// The CPU-activity clock (third signal) vetoes the net for a busy-but-silent
    /// agent. With a stale commit and no pane transcript, the git+fs clocks alone
    /// would trip the net; a FRESH `extra_activity_unix` (CPU still advancing)
    /// must hold it off. Regression for the `/llm-review` "silent long-running
    /// subprocess false-positive" finding — the guard the done-criteria demands.
    #[test]
    fn node_idle_unmerged_cpu_clock_vetoes_busy_but_silent() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Ancient commit; no `agent.log` → only the commit clock is available.
        let (repo, wt, base) = init_unmerged_repo_at(&tmp, Some("2020-01-01T00:00:00 +0000"));
        let paths = setup_unmerged_run(&tmp, &repo, &wt, &base, 1, "spinoff", "autonomous");
        let n = n0001(&paths);
        let git = cleanup::git_bin();
        // Far-future "now" so the ancient commit reads as long-idle.
        let now = 4_000_000_000i64;
        // Without a CPU clock, the stale commit alone trips the net.
        assert!(
            cleanup::node_idle_unmerged(&paths, &n, &git, now, 1800, None).is_some(),
            "stale commit alone should trip the net"
        );
        // With a FRESH CPU clock (busy but silent), the net must NOT fire.
        assert!(
            cleanup::node_idle_unmerged(&paths, &n, &git, now, 1800, Some(now)).is_none(),
            "a fresh CPU-activity clock must veto the idle verdict"
        );
        let _ = repo;
        let _ = wt;
    }

    /// `cpu_activity_clock`: first observation seeds "active now"; a changed CPU
    /// total advances the change-time; a flat total keeps the old change-time (so
    /// the clock ages and eventually lets the net fire); an unreadable CPU
    /// (`None`) contributes no clock at all.
    #[test]
    fn cpu_activity_clock_tracks_changes() {
        let mut map = std::collections::BTreeMap::new();
        // First observation at t=100 → seeded to now (assume active).
        assert_eq!(cpu_activity_clock(&mut map, "n", Some(50), 100), Some(100));
        // Same CPU total at t=200 → change-time stays 100 (idle since 100).
        assert_eq!(cpu_activity_clock(&mut map, "n", Some(50), 200), Some(100));
        // CPU advanced at t=300 → change-time jumps to 300 (active).
        assert_eq!(cpu_activity_clock(&mut map, "n", Some(60), 300), Some(300));
        // Flat again at t=400 → stays 300.
        assert_eq!(cpu_activity_clock(&mut map, "n", Some(60), 400), Some(300));
        // Unreadable CPU → no clock contributed (fall back to other clocks).
        assert_eq!(cpu_activity_clock(&mut map, "n", None, 500), None);
    }

    // --- Bounded auto-retry on empty-handed agent-died (issue
    // `autoretry-agent-died-worker`) ------------------------------------------

    /// RAII env guard: set on construct, restore prior value / unset on drop, so a
    /// panicking assertion cannot leak the var into another test. Callers hold
    /// `GRACE_ENV_LOCK` for the duration (these tests are `#[serial]`).
    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, val);
            Self { key, old }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Init a repo on `main` with one commit, then fork `wt/foo` at `main` with NO
    /// commits of its own — the empty-handed case (`main..wt/foo == 0`). Returns
    /// `(repo, worktree, base_sha)` where `base_sha == main == wt HEAD`.
    fn init_empty_handed_repo(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf, String) {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        tgit(&repo, &["init", "-q", "-b", "main"]);
        tgit(&repo, &["config", "user.email", "t@example.com"]);
        tgit(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        tgit(&repo, &["add", "-A"]);
        tgit(&repo, &["commit", "-qm", "init"]);
        let base = trev(&repo, "main");
        let wt = tmp.path().join("wt");
        tgit(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt/foo",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        (repo, wt, base)
    }

    /// Like [`init_empty_handed_repo`], but the worktree carries ONE commit ahead
    /// of `main` that is NOT merged — the recoverable / committed-work case
    /// (`main..wt/foo == 1`). Salvage territory, never retry.
    fn init_committed_repo(tmp: &tempfile::TempDir) -> (PathBuf, PathBuf, String) {
        let (repo, wt, base) = init_empty_handed_repo(tmp);
        std::fs::write(wt.join("fix.rs"), "work").unwrap();
        tgit(&wt, &["add", "-A"]);
        tgit(&wt, &["commit", "-qm", "agent work"]);
        (repo, wt, base)
    }

    /// A run dir + autonomous single-node worker manifest + one `n-0001` node
    /// pointing at `wt` (branch `wt/foo`, base `base`), with `agent_pid` set so the
    /// watchdog reads a real liveness verdict. `source_repo`/`source_branch` are
    /// recorded so the empty-handed check and the re-spawn resolve the right repo.
    fn setup_autonomous_run(
        tmp: &tempfile::TempDir,
        repo: &Path,
        wt: &Path,
        base: &str,
        agent_pid: i32,
    ) -> RunPaths {
        let run_id = "01jxwd0000000000000000000w";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_repo": repo.to_str().unwrap(),
                "source_branch": "main",
            }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.created",
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            json!({
                "kind": "spinoff",
                "branch": "wt/foo",
                "base_sha": base,
                "worktree_path": wt.to_str().unwrap(),
                "agent_pid": agent_pid,
            }),
        )
        .unwrap();
        paths
    }

    /// Write a stub `create.sh` that materializes a real worktree (forked from the
    /// requested `--base` in `repo`) for the requested branch, spawns a long-lived
    /// `sleep` agent, and emits the `SpawnOutcome` envelope. `repo` and `wt_root`
    /// are baked into the script so no env plumbing is needed.
    fn write_respawn_stub(scratch: &Path, repo: &Path, wt_root: &Path) -> PathBuf {
        let p = scratch.join("respawn-create.sh");
        let body = format!(
            r#"#!/bin/bash
set -e
base=main
args=("$@")
for ((i=0; i<${{#args[@]}}; i++)); do
  if [ "${{args[$i]}}" = "--base" ]; then base="${{args[$((i+1))]}}"; fi
done
branch="${{args[$((${{#args[@]}}-2))]}}"
safe=$(printf '%s' "$branch" | tr '/' '_')
wt="{wt_root}/$safe"
git -C "{repo}" worktree add -q -b "$branch" "$wt" "$base" >/dev/null 2>&1
bash -c 'exec sleep 120' </dev/null >/dev/null 2>&1 &
agent_pid=$!
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"$branch","worktree_path":"$wt","tmux_window":"$branch","agent_pid_hint":$agent_pid,"tmux_socket":null,"tmux_session":null,"tmux_window_id":null}}
EOF
"#,
            wt_root = wt_root.display(),
            repo = repo.display(),
        );
        std::fs::write(&p, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    /// Done-criterion: an autonomous single-node worker that dies EMPTY-HANDED is
    /// re-spawned on a clean worktree at the run's source branch (a durable
    /// `node.retry` event, `retry_attempts` incremented, node NOT terminalized),
    /// and the re-spawned worker then self-merges → the run reconciles to SUCCESS.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn empty_handed_death_retries_and_then_succeeds() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        // Share the create.sh env lock with `run::spawn::tests` so our global
        // `OCTL_CREATE_SH` mutation cannot race their fixtures.
        let _create_lock = crate::run::spawn::tests::ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _backoff = EnvGuard::set(AGENT_RETRY_BACKOFF_ENV, "0");
        let _tmux = EnvGuard::set("TMUX_BIN", "/nonexistent/tmux");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_empty_handed_repo(&tmp);
        // A definitely-dead pid: spawn `true`, reap it, reuse its pid.
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_autonomous_run(&tmp, &repo, &wt, &base, dead_pid);
        std::fs::write(paths.root.join("prompt.md"), "do the thing").unwrap();

        let wt_root = tmp.path().join("respawn-wts");
        std::fs::create_dir_all(&wt_root).unwrap();
        let stub = write_respawn_stub(tmp.path(), &repo, &wt_root);
        let _create = EnvGuard::set("OCTL_CREATE_SH", stub.to_str().unwrap());

        let mut streak = std::collections::BTreeMap::new();
        let mut retries = std::collections::BTreeMap::new();
        // One tick with backoff 0: detect death → park → reconcile → re-spawn.
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        assert!(
            !n.status.is_terminal(),
            "a retried node is NOT terminalized"
        );
        assert_eq!(n.retry_attempts, 1, "one retry recorded on the node");
        assert_eq!(
            n.branch.as_deref(),
            Some("wt/foo-r1"),
            "rewired to fresh branch"
        );
        assert!(
            retries.is_empty(),
            "park cleared after a successful re-spawn"
        );
        let has_retry_event = std::fs::read_to_string(paths.events())
            .unwrap()
            .lines()
            .any(|l| l.contains("\"node.retry\""));
        assert!(has_retry_event, "a durable node.retry event is emitted");
        let new_pid = n.agent_pid.expect("re-spawned agent pid");
        assert!(
            pid_file::pid_alive(new_pid as u32),
            "the re-spawned agent is alive"
        );
        assert!(
            !tbranch_exists(&repo, "wt/foo"),
            "the stale empty-handed branch is torn down"
        );

        // Eventual success: the re-spawned worker commits and self-merges; a later
        // tick reconciles the merged branch to terminal SUCCESS.
        let new_wt = PathBuf::from(n.worktree_path.clone().unwrap());
        std::fs::write(new_wt.join("fix.rs"), "done").unwrap();
        tgit(&new_wt, &["add", "-A"]);
        tgit(&new_wt, &["commit", "-qm", "retried work"]);
        tgit(&repo, &["merge", "--ff-only", "wt/foo-r1"]);

        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();
        let n2 = n0001(&paths);
        let report = n2.last_report.expect("terminal report after merge");
        assert_eq!(
            report["success"], true,
            "the re-spawned worker's merge reconciles to success"
        );
        assert!(n2.status.is_terminal());

        let _ = PCommand::new("kill").arg(new_pid.to_string()).status();
    }

    /// Done-criterion: a persistently-dying empty-handed worker terminalizes
    /// `failed` after the bounded attempts are exhausted — never an unbounded
    /// respawn loop. Staged at `retry_attempts == max` (one prior retry recorded),
    /// so the next empty-handed death crosses the budget and writes the terminal
    /// failed report, stamped with the retry count.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn empty_handed_death_terminalizes_failed_after_max_attempts() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _max = EnvGuard::set(AGENT_RETRY_MAX_ATTEMPTS_ENV, "1");
        let _backoff = EnvGuard::set(AGENT_RETRY_BACKOFF_ENV, "0");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_empty_handed_repo(&tmp);
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_autonomous_run(&tmp, &repo, &wt, &base, dead_pid);

        // Pre-stage one prior retry (attempt 1 == max): the node's durable bound is
        // now exhausted, and it still points at the dead pid + empty-handed branch.
        let nid = NodeId::parse_str("n-0001").unwrap();
        append_and_apply_event(
            &paths,
            "node.retry",
            Some(&nid),
            None,
            json!({
                "attempt": 1,
                "reason": "agent-died",
                "branch": "wt/foo",
                "base_sha": base,
                "worktree_path": wt.to_str().unwrap(),
                "agent_pid": dead_pid,
            }),
        )
        .unwrap();
        assert_eq!(n0001(&paths).retry_attempts, 1, "pre-staged at the budget");

        let mut streak = std::collections::BTreeMap::new();
        let mut retries = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        let n = n0001(&paths);
        let report = n.last_report.expect("terminal failed report");
        assert_eq!(report["success"], false, "exhausted budget → failed");
        assert_eq!(report["reason"], "agent-died");
        assert_eq!(
            report["retry_attempts"], 1,
            "the failed report records how many retries preceded it"
        );
        assert!(
            n.status.is_terminal(),
            "run is terminalized, not respun forever"
        );
        assert!(
            retries.is_empty(),
            "no park scheduled once the budget is exhausted"
        );
    }

    /// Retry ⟂ salvage: a death that left COMMITTED work (branch ahead of source)
    /// is NEVER auto-retried — the salvage path owns it. The watchdog terminalizes
    /// `failed` with the `recoverable_work` signal (not a retry), and no park is
    /// scheduled, so a re-spawn from base can never clobber the committed branch.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn committed_work_death_is_not_retried_and_preserves_salvage_signal() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _backoff = EnvGuard::set(AGENT_RETRY_BACKOFF_ENV, "0");
        let _tmux = EnvGuard::set("TMUX_BIN", "/nonexistent/tmux");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_committed_repo(&tmp);
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_autonomous_run(&tmp, &repo, &wt, &base, dead_pid);

        let mut streak = std::collections::BTreeMap::new();
        let mut retries = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        assert!(
            retries.is_empty(),
            "committed work is NOT parked for retry (salvage wins)"
        );
        let n = n0001(&paths);
        let report = n.last_report.expect("terminal failed report");
        assert_eq!(report["success"], false);
        assert!(
            report.get("recoverable_work").is_some(),
            "the salvage signal is preserved on the failed report"
        );
        assert!(
            report.get("retry_attempts").is_none(),
            "a committed-work death is not a retry failure"
        );
        assert_eq!(n.retry_attempts, 0, "no retry was attempted");
        // The committed branch and worktree are untouched (salvage owns them).
        assert!(
            tbranch_exists(&repo, "wt/foo"),
            "committed branch preserved"
        );
        assert!(wt.exists(), "committed worktree preserved");
    }

    /// A stub `create.sh` that ALWAYS fails (exit 1 + error envelope), for the
    /// spawn-infrastructure-failure budget test.
    fn write_failing_stub(scratch: &Path) -> PathBuf {
        let p = scratch.join("failing-create.sh");
        let body = "#!/bin/bash\n\
             echo '{\"schema_version\":1,\"error\":{\"code\":\"stub-boom\",\"message\":\"stub always fails\"}}' >&2\n\
             exit 1\n";
        std::fs::write(&p, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    /// Done-criterion: a broken spawn INFRASTRUCTURE (every `create.sh` fails) is
    /// also bounded — after `AGENT_RESPAWN_MAX_FAILURES` consecutive failures the
    /// run terminalizes `failed` (reason `agent-respawn-failed`), never looping
    /// forever. The stale worktree survives each failed spawn (spawn-before-teardown),
    /// so the empty-handed re-verify keeps holding and the budget is real.
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn respawn_infrastructure_failure_terminalizes_after_budget() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _create_lock = crate::run::spawn::tests::ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _backoff = EnvGuard::set(AGENT_RETRY_BACKOFF_ENV, "0");
        let _failures = EnvGuard::set(AGENT_RESPAWN_MAX_FAILURES_ENV, "2");
        let _tmux = EnvGuard::set("TMUX_BIN", "/nonexistent/tmux");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_empty_handed_repo(&tmp);
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_autonomous_run(&tmp, &repo, &wt, &base, dead_pid);
        std::fs::write(paths.root.join("prompt.md"), "do the thing").unwrap();
        let stub = write_failing_stub(tmp.path());
        let _create = EnvGuard::set("OCTL_CREATE_SH", stub.to_str().unwrap());

        let mut streak = std::collections::BTreeMap::new();
        let mut retries = std::collections::BTreeMap::new();

        // Tick 1: death → park → reconcile → create.sh fails (failures=1 < 2). Node
        // stays non-terminal, park retained; the stale branch survives.
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert!(
            !n0001(&paths).status.is_terminal(),
            "one failure is not terminal"
        );
        assert!(
            retries.contains_key("n-0001"),
            "park retained after 1 failure"
        );
        assert!(
            tbranch_exists(&repo, "wt/foo"),
            "stale branch survives a failed spawn"
        );

        // Tick 2: reconcile → create.sh fails again (failures=2 == budget) → terminalize.
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();
        let n = n0001(&paths);
        let report = n.last_report.expect("terminal failed report after budget");
        assert_eq!(report["success"], false);
        assert_eq!(report["reason"], "agent-respawn-failed");
        assert!(
            n.status.is_terminal(),
            "run terminalizes, never loops forever"
        );
        assert!(retries.is_empty(), "park dropped once terminalized");
    }

    /// retry ⟂ salvage / no-data-loss: a death whose worktree holds UNCOMMITTED
    /// work (dirty tree, zero commits ahead) is NOT empty-handed, so it is never
    /// retried — the destructive teardown-and-respawn can never discard that work.
    /// It falls through to the terminal `agent-died` report (whose blocked-handoff
    /// gate preserves the worktree).
    #[test]
    #[serial_test::serial(octl_watchdog_grace)]
    fn dirty_worktree_death_is_not_retried() {
        let _lock = GRACE_ENV_LOCK.lock().unwrap();
        let _grace = GraceGuard::zero();
        let _backoff = EnvGuard::set(AGENT_RETRY_BACKOFF_ENV, "0");
        let _tmux = EnvGuard::set("TMUX_BIN", "/nonexistent/tmux");
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt, base) = init_empty_handed_repo(&tmp);
        // Uncommitted (untracked) work in the worktree: 0 commits ahead, dirty tree.
        std::fs::write(wt.join("scratch.rs"), "wip, not committed").unwrap();
        let dead = PCommand::new("true").spawn().unwrap();
        let dead_pid = dead.id() as i32;
        let mut dead = dead;
        dead.wait().unwrap();
        let paths = setup_autonomous_run(&tmp, &repo, &wt, &base, dead_pid);

        let mut streak = std::collections::BTreeMap::new();
        let mut retries = std::collections::BTreeMap::new();
        watchdog_tick(
            &paths,
            &mut streak,
            &mut retries,
            &mut std::collections::BTreeMap::new(),
        )
        .unwrap();

        assert!(
            retries.is_empty(),
            "a dirty worktree is NOT parked for retry"
        );
        let n = n0001(&paths);
        let report = n.last_report.expect("terminal failed report");
        assert_eq!(report["success"], false);
        assert_eq!(report["reason"], "agent-died");
        assert_eq!(n.retry_attempts, 0, "no retry attempted");
        assert!(
            wt.join("scratch.rs").exists(),
            "uncommitted work not destroyed"
        );
    }
}
