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

pub mod cleanup;
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
use crate::run::{from_core, parse_run_id, run_paths};

/// Polling cadences (design.md §7.5 defaults).
const TAIL_TICK: Duration = Duration::from_millis(500);
const WATCHDOG_TICK: Duration = Duration::from_secs(1);
/// Max time we wait for a spawned child run's directory to appear
/// (handoff D1).
const CHILD_DIR_WAIT: Duration = Duration::from_secs(5);
/// Consecutive missing-manifest polls (`WATCHDOG_TICK` apart, so ≈3s)
/// before we self-terminate. Defends against orphaning: when a run dir is
/// deleted out from under us (a test's `TempDir` on teardown, or an
/// operator removing the run), there is nothing left to supervise and
/// polling the vanished directory forever wastes CPU + file descriptors.
/// We require a short streak rather than reacting to a single missed read
/// so a transient `stat` hiccup cannot kill a live supervisor.
const SELF_TERMINATE_TICKS: u32 = 3;

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

pub fn dispatch(
    args: SuperviseArgs,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let run_id = args.run_id.clone();
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(CliError {
            kind: ExitKind::User,
            code: "run_not_found".into(),
            message: format!("no run with id {run_id}"),
            invalid_value: Some(run_id),
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

    let mut state = state::load(&paths.root)?;
    let mut own_tail = tail::EventTail::new(paths.events(), state.last_seq_own);
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

    // Per-node count of consecutive ticks a node has presented the
    // `TmuxGone` half-state. §7.5 requires half-states to "resolve via
    // short retry then commit to dead" — we only synthesize a terminal
    // report once the streak crosses `HALF_STATE_TICKS`. Unambiguous
    // `Dead` / `Recycled` verdicts are committed immediately.
    let mut half_state_streak: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();

    // Consecutive ticks our run's manifest.json has been missing. Reset
    // to 0 on any tick where it exists; once it crosses
    // `SELF_TERMINATE_TICKS` we self-terminate (run dir vanished).
    let mut manifest_missing_streak: u32 = 0;

    // Whether terminal-transition cleanup (tmux window close + worktree
    // remove + branch delete, autonomous kinds only) has already run this
    // process. Performed once when the run is first observed terminal; the
    // steps are idempotent/lenient anyway, but the flag avoids re-shelling
    // out every tick between the terminal transition and the loop exit.
    let mut cleaned = false;

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
                    // tracking set is the single arbiter, §7.2).
                    if state.spawned_children.contains_key(&child_run_id) {
                        continue;
                    }
                    match spawn_child_supervisor(&root, &child_run_id, &paths) {
                        Ok(child_pid) => {
                            state
                                .spawned_children
                                .insert(child_run_id.clone(), child_pid);
                        }
                        Err(e) => {
                            warn!(
                                target: "orchestratectl::supervise",
                                child = %child_run_id,
                                error = %e.message,
                                "child spawn failed (tail still open; reports will be consumed)"
                            );
                            // Record on parent log so a future
                            // operator can see the failure (D1).
                            let _ = append_and_apply_event(
                                &paths,
                                "child.spawn_failed",
                                ev.node_id.as_ref(),
                                None,
                                json!({
                                    "child_run_id": child_run_id,
                                    "reason": e.message,
                                }),
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

        // Loop 3: watchdog. We don't yet have a generalized agent
        // registry (that's `all-kinds-spawn`'s territory). The current
        // surface exercises liveness for any node that carries an
        // `agent_pid` recorded by `create.sh` integration.
        if let Err(e) = watchdog_tick(&paths, &mut half_state_streak) {
            warn!(
                target: "orchestratectl::supervise",
                error = %e.message,
                "watchdog tick failed"
            );
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
        if !cleaned {
            if let Ok(Some(m)) = read_manifest_opt(&paths) {
                if m.status.is_terminal() {
                    let warranted = m.kind.lifecycle() == Lifecycle::Autonomous
                        || cleanup::any_node_merged_explicitly(&paths);
                    if warranted {
                        cleanup::cleanup_terminal_nodes(&paths);
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
        // remains non-terminal, we're done.
        if all_work_done(&paths, &child_tails) {
            break "work-complete";
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

fn spawn_child_supervisor(
    root: &Path,
    child_run_id: &str,
    parent_paths: &RunPaths,
) -> Result<u32, CliError> {
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

    // Fork+exec a fully-detached child supervisor (setsid + double-fork via
    // `supervisor_spawn`). The grandchild is reparented to init, so an exited
    // child supervisor never becomes a zombie under this long-lived parent
    // (and `kill(pid, 0)` never misreports a zombie as alive, which would
    // corrupt the PID-staleness check). `RUST_LOG_NOSPAWN` is cleared because
    // it is reserved for tests.
    let stderr_path: PathBuf = child_dir.join("supervisor.stderr.log");
    let mut cmd =
        crate::run::supervisor_spawn::detached_supervise_command(child_run_id, &stderr_path)?;
    cmd.env_remove("RUST_LOG_NOSPAWN");
    crate::run::supervisor_spawn::spawn_and_reap(&mut cmd, child_run_id)?;

    // The grandchild is launched and detached; that — not the pid file — is
    // the success signal, so we never block the single-threaded parent tick
    // waiting for it to boot. We do a SINGLE non-blocking, identity-verified
    // read of the child's own `supervisor.pid` (the authoritative record the
    // child writes under its run flock during `claim_pid_atomic`). It is
    // usually not there yet — that is fine: the cosmetic `supervisor_pid`
    // node record and the `child.supervisor_attached` event are only emitted
    // once a real pid is readable; the durable truth is the child's pid file.
    let child_paths = run_paths(root, child_run_id)?;
    let pid = crate::run::supervisor_spawn::read_live_recorded_pid(&child_paths).unwrap_or(0);

    if pid != 0 {
        // Best-effort: record supervisor_pid on the child's root node. This
        // read-modify-write races the child supervisor's own boot writes, so
        // it must be done under the child run's flock (F11) — without it the
        // last writer silently clobbers the other's fields.
        match RunLock::acquire(&child_paths.lock()) {
            Ok(_guard) => {
                // The child run's root node is always `n-0001` (a static, valid id).
                let root_node = NodeId::parse_str("n-0001").expect("n-0001 is a valid node id");
                if let Ok(Some(mut n)) = read_node_opt(&child_paths, &root_node) {
                    n.supervisor_pid = Some(pid as i32);
                    let _ = octl_core::write_node(&child_paths, &n);
                }
            }
            Err(e) => {
                // Non-fatal: the `child.supervisor_attached` event below records
                // the attach, so this projection write is a convenience.
                warn!(
                    target: "orchestratectl::supervise",
                    child = %child_run_id,
                    error = %e,
                    "could not lock child run to record supervisor_pid"
                );
            }
        }
        // Record the attach on the parent log, but only with a real pid —
        // never emit `supervisor_pid: 0`, which would be a false "attached".
        let _ = append_and_apply_event(
            parent_paths,
            "child.supervisor_attached",
            None,
            None,
            json!({"child_run_id": child_run_id, "supervisor_pid": pid}),
        );
    }
    info!(
        target: "orchestratectl::supervise",
        child = %child_run_id,
        pid,
        "spawned child supervisor"
    );
    Ok(pid)
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

fn watchdog_tick(
    paths: &RunPaths,
    half_state_streak: &mut std::collections::BTreeMap<String, u32>,
) -> Result<(), CliError> {
    let now = Utc::now();
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

    for (node_id, nid, n, probe) in candidates {
        let v = watchdog::check_liveness(&probe, &tmux_snapshot);
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
        if commit && n.last_report.is_none() {
            // The `n.last_report.is_none()` we just tested was read OUTSIDE the
            // run lock, so a real `node.report` could land in the window
            // between that read and this synthesis. Acquire the run flock and
            // re-read the node projection under it before committing: only
            // synthesize if the node is STILL non-terminal and STILL
            // unreported. This closes the F15 duplicate-terminal-report race
            // (the parent-side reducer dedups it anyway, but we avoid emitting
            // the second event at all). `append_and_apply_unlocked` is the
            // sanctioned lock-held path — re-entering `append_and_apply_event`
            // here would deadlock on the flock.
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
            // Synthesize a terminal node.report under the held run flock.
            let data = json!({
                "success": false,
                "failed": true,
                "cancelled": false,
                "reason": v.reason(),
                "summary": format!("Agent for node {} stopped responding: {}", node_id, v.reason()),
                "discussion_items": [],
                "spinoff_proposals": [],
                "wrap_up_recommendations": [],
            });
            if let Err(e) = append_and_apply_unlocked(paths, "node.report", Some(&nid), None, data)
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
    }
    // Drop streaks for every node that did NOT present `TmuxGone` this
    // tick (committed, recovered, terminal, or no longer scanned) so the
    // count is strictly consecutive and the map cannot grow unbounded.
    half_state_streak.retain(|k, _| tmux_gone_this_tick.contains(k));
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
}
