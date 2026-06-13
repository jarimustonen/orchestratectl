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
//! Lifecycle: trap SIGINT/SIGTERM via `ctrlc`, refuse to launch if the
//! `<run-dir>/supervisor.pid` PID is alive, atomically write our own
//! PID on boot, emit `supervisor.exited` and remove the PID file on
//! exit. `--once` and `--max-iter <n>` are test-only escape hatches.

pub mod pid_file;
pub mod reducer;
pub mod state;
pub mod tail;
pub mod watchdog;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Args as ClapArgs;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use octl_core::{append_and_apply, read_manifest_opt, read_node_opt, RunPaths, Status};

use crate::error::{CliError, ExitKind};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, require_safe_id, run_paths};

/// Polling cadences (design.md §7.5 defaults).
const TAIL_TICK: Duration = Duration::from_millis(500);
const WATCHDOG_TICK: Duration = Duration::from_millis(1000);
/// Max time we wait for a spawned child run's directory to appear
/// (handoff D1).
const CHILD_DIR_WAIT: Duration = Duration::from_secs(5);

#[derive(ClapArgs, Debug)]
pub struct SuperviseArgs {
    /// Run id to supervise.
    pub run_id: String,
    /// Tick the watchdog + tail loops exactly once, then exit cleanly.
    /// **Test-only escape hatch — never set in production.**
    #[arg(long)]
    pub once: bool,
    /// Tick at most this many iterations, then exit cleanly. Combine
    /// with `--once` to cap a self-bounded run. **Test-only escape
    /// hatch — never set in production.**
    #[arg(long)]
    pub max_iter: Option<u32>,
}

pub fn dispatch(args: SuperviseArgs, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);
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
    if let Some(existing) = pid_file::read_pid(&pid_path) {
        if pid_file::pid_alive(existing) {
            return Err(CliError {
                kind: ExitKind::System,
                code: "supervisor_already_running".into(),
                message: format!(
                    "supervisor pid {existing} for run {run_id} is alive (kill it or use `run reattach`)",
                ),
                invalid_value: None,
                expected: None,
            });
        }
        // Stale PID file: log and overwrite.
        warn!(
            target: "orchestratectl::supervise",
            stale_pid = existing,
            "removing stale supervisor.pid"
        );
    }

    let our_pid = std::process::id();
    pid_file::write_pid(&pid_path, our_pid)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&shutdown);
    if let Err(e) = ctrlc::set_handler(move || {
        s2.store(true, Ordering::SeqCst);
    }) {
        // A previously-installed handler isn't fatal; just warn.
        warn!(
            target: "orchestratectl::supervise",
            error = %e,
            "could not install signal handler"
        );
    }

    info!(
        target: "orchestratectl::supervise",
        run_id = %run_id,
        pid = our_pid,
        "supervisor started"
    );
    let _ = append_and_apply(
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
    // Reseed children we already spawned in a previous incarnation.
    for (cid, _) in state.spawned_children.clone() {
        let child_paths = run_paths(&root, &cid);
        let events = child_paths.events();
        let seq = state.last_seq_by_child.get(&cid).copied().unwrap_or(0);
        child_tails.insert(
            cid.clone(),
            ChildTracking {
                root: child_paths.root,
                tail: tail::EventTail::new(events, seq),
                terminal: false,
            },
        );
    }

    let mut iter: u32 = 0;
    let exit_reason: &'static str = loop {
        if shutdown.load(Ordering::SeqCst) {
            break "signal";
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
                    if state.spawned_children.contains_key(&child_run_id) {
                        continue;
                    }
                    match spawn_child_supervisor(&root, &child_run_id, &paths) {
                        Ok(child_pid) => {
                            state
                                .spawned_children
                                .insert(child_run_id.clone(), child_pid);
                            let child_paths = run_paths(&root, &child_run_id);
                            let events = child_paths.events();
                            child_tails.insert(
                                child_run_id.clone(),
                                ChildTracking {
                                    root: child_paths.root,
                                    tail: tail::EventTail::new(events, 0),
                                    terminal: false,
                                },
                            );
                        }
                        Err(e) => {
                            warn!(
                                target: "orchestratectl::supervise",
                                child = %child_run_id,
                                error = %e.message,
                                "child spawn failed"
                            );
                            // Record on parent log so a future
                            // operator can see the failure (D1).
                            let _ = append_and_apply(
                                &paths,
                                "child.spawn_failed",
                                ev.node_id.as_deref(),
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
                            // Save cursor before bailing out.
                            let _ = state::save(&paths.root, &state);
                            // Terminal status on our own run is the
                            // single signal that we should wrap up.
                            // We break out of the inner for-loop here
                            // and let the outer loop's idle check run.
                        }
                    }
                }
                _ => {}
            }
        }

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
                        let child_node_id =
                            ev.node_id.clone().unwrap_or_else(|| "n-0001".to_string());
                        // Discover the parent's spawning node by
                        // scanning our own nodes/ for a child entry.
                        // Default to "n-0001" if not found — this is
                        // the standard top-level root node.
                        let parent_node_id = find_spawning_node(&paths, &cid)
                            .unwrap_or_else(|| "n-0001".to_string());
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
                            }
                            Ok(None) => {}
                            Err(e) => {
                                warn!(
                                    target: "orchestratectl::supervise",
                                    error = %e.message,
                                    "node.report consumption failed"
                                );
                            }
                        }
                        entry.terminal = true;
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
        }

        // Loop 3: watchdog. We don't yet have a generalized agent
        // registry (that's `all-kinds-spawn`'s territory). The current
        // surface exercises liveness for any node that carries an
        // `agent_pid` recorded by `create.sh` integration.
        if let Err(e) = watchdog_tick(&paths) {
            warn!(
                target: "orchestratectl::supervise",
                error = %e.message,
                "watchdog tick failed"
            );
        }

        // Persist cursors after each tick so a crash mid-run loses at
        // most one tick of progress (and the deterministic-ID reducer
        // makes that loss idempotent anyway).
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

    // Clean shutdown.
    let _ = state::save(&paths.root, &state);
    let _ = append_and_apply(
        &paths,
        "supervisor.exited",
        None,
        None,
        json!({"pid": our_pid, "reason": exit_reason}),
    )
    .map_err(from_core);
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
                "supervisor exited run={} pid={} reason={} iter={}",
                run_id, our_pid, exit_reason, iter
            );
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

struct ChildTracking {
    #[allow(dead_code)]
    root: PathBuf,
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
    let child_dir = octl_core::run_dir(root, child_run_id);
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

    // Build a log path under <child-dir>/supervisor.stderr.log.
    let stderr_path: PathBuf = child_dir.join("supervisor.stderr.log");
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .map_err(|e| {
            CliError::system("io_error", format!("open {}: {}", stderr_path.display(), e))
        })?;
    let stderr_clone = stderr_file
        .try_clone()
        .map_err(|e| CliError::system("io_error", format!("dup fd: {e}")))?;

    let exe = std::env::current_exe()
        .map_err(|e| CliError::system("io_error", format!("current_exe: {e}")))?;
    let child = Command::new(exe)
        .arg("supervise")
        .arg(child_run_id)
        .stdout(stderr_file)
        .stderr(stderr_clone)
        .env_remove("RUST_LOG_NOSPAWN") // reserved for tests
        .spawn()
        .map_err(|e| {
            CliError::system(
                "spawn_failed",
                format!("spawn supervise {}: {}", child_run_id, e),
            )
        })?;
    let pid = child.id();
    // Best-effort: record supervisor_pid on the child's root node.
    let child_paths = run_paths(root, child_run_id);
    if let Ok(Some(mut n)) = read_node_opt(&child_paths, "n-0001") {
        n.supervisor_pid = Some(pid as i32);
        let _ = octl_core::write_node(&child_paths, &n);
    }
    // Record on the parent's tracking node too via an event.
    let _ = append_and_apply(
        parent_paths,
        "child.supervisor_attached",
        None,
        None,
        json!({"child_run_id": child_run_id, "supervisor_pid": pid}),
    );
    info!(
        target: "orchestratectl::supervise",
        child = %child_run_id,
        pid,
        "spawned child supervisor"
    );
    Ok(pid)
}

/// Find which of our own nodes registered `child_run_id` in its
/// `children` list. Scan `<paths.root>/nodes/`.
fn find_spawning_node(paths: &RunPaths, child_run_id: &str) -> Option<String> {
    let entries = std::fs::read_dir(paths.nodes_dir()).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let stem = p.file_stem().and_then(|s| s.to_str()).map(str::to_string);
        let Some(node_id) = stem else { continue };
        if let Ok(Some(n)) = read_node_opt(paths, &node_id) {
            if n.children.iter().any(|c| c.run_id == child_run_id) {
                return Some(node_id);
            }
        }
    }
    None
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

fn watchdog_tick(paths: &RunPaths) -> Result<(), CliError> {
    // Scan our own nodes/ for any with an `agent_pid` that is running.
    // If a node is non-terminal and its agent has died (per dual-poll
    // protocol) AND it has not already produced a `node.report`,
    // synthesize one with `failed: true, reason: "agent-died"`.
    let entries = match std::fs::read_dir(paths.nodes_dir()) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", paths.nodes_dir().display(), e),
            ));
        }
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(node_id) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(Some(n)) = read_node_opt(paths, &node_id) else {
            continue;
        };
        if matches!(n.status, Status::Done | Status::Failed | Status::Cancelled) {
            continue;
        }
        let Some(pid) = n.agent_pid else { continue };
        let probe = watchdog::AgentProbe {
            pid: pid as u32,
            start_time: n.agent_pid_start_time.map(|t| t.timestamp().max(0) as u64),
            tmux_window: n.tmux_window.clone(),
            // Heuristic: if tmux_window isn't recorded we can't probe
            // tmux. Don't fail liveness on that absence alone.
            skip_tmux_check: n.tmux_window.is_none(),
        };
        let v = watchdog::check_liveness(&probe);
        if v.is_terminal() && n.last_report.is_none() {
            // Synthesize a terminal node.report under the run's flock.
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
            if let Err(e) = append_and_apply(paths, "node.report", Some(&node_id), None, data) {
                warn!(
                    target: "orchestratectl::supervise",
                    node = %node_id,
                    error = %e,
                    "synthesize node.report failed"
                );
            }
        }
    }
    Ok(())
}
