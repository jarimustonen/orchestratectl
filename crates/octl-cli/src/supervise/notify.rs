//! Terminal-completion notification hook (`run create --notify <cmd>`).
//!
//! When a run reaches a terminal state (`done | failed | cancelled`) the
//! supervisor runs the caller-registered command exactly once, BEFORE the
//! teardown path removes the worktree/window/branch. This is the push signal
//! a spawning session needs so it can learn of completion without polling
//! (issue `no-completion-notification-to-parent`): the parent registers e.g.
//! a line-append to a file its harness watches, a `terminal-notifier` /
//! `notify-send` desktop toast, or a FIFO write.
//!
//! ## At-most-once
//!
//! Firing is gated on a durable `run.notified` marker event carrying the
//! deterministic idempotency key `supervisor-notify:<run-id>`. The marker is
//! appended (and fsynced) BEFORE the command is spawned, so a supervisor
//! crash/restart replays it as an idempotent no-op and never re-fires. The
//! trade-off is deliberate: at-most-once (a crash in the narrow window between
//! the marker append and the spawn drops the notification) is safer than
//! at-least-once (which could spam a parent's inbox on every restart tick).
//!
//! ## Non-blocking
//!
//! The command is spawned detached (`sh -c`, no `wait`) so a slow or hung hook
//! can never wedge the single-threaded supervisor tick. The supervisor is
//! about to wind down anyway; a still-running hook is reparented to init.

use serde_json::{json, Value};
use tracing::{info, warn};

use octl_core::{read_node_opt, NodeId, RunLock, RunPaths, Status};

use crate::run::from_core;

/// Reporting node whose terminal `node.report` carries the run's outcome
/// summary. Every single-worker worktree kind has exactly one node
/// (`n-0001`); mirrors `run wait`'s and `run merge`'s `DEFAULT_NODE_ID`.
const DEFAULT_NODE_ID: &str = "n-0001";

/// Fire the run's `--notify` hook once, if one is registered and this is the
/// first time the run is observed terminal.
///
/// `status` is the run's terminal manifest status; `kind`/`title` are surfaced
/// to the hook for a richer message. Best-effort throughout: any failure is
/// logged and swallowed — a broken notification must never block teardown or
/// crash the supervisor. Returns without doing anything when `notify_cmd` is
/// `None`.
pub fn maybe_fire(
    paths: &RunPaths,
    run_id: &str,
    notify_cmd: Option<&str>,
    status: Status,
    kind: &str,
    title: &str,
) {
    let Some(cmd) = notify_cmd else {
        return;
    };

    let summary = read_summary(paths).unwrap_or_default();
    let status_str = status_kebab(status);

    // Durable at-most-once gate: record the marker (fsynced) BEFORE spawning.
    // A prior tick / a pre-restart supervisor that already recorded it makes
    // this an idempotent replay, and we do NOT re-fire.
    let key = format!("supervisor-notify:{run_id}");
    let appended = match octl_core::append_and_apply_event(
        paths,
        "run.notified",
        None,
        Some(&key),
        json!({ "status": status_str, "notify_cmd": cmd }),
    ) {
        Ok(res) => !res.idempotent_replay,
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                run_id = %run_id,
                error = %e,
                "could not record run.notified marker; skipping notify hook (will retry next tick)"
            );
            return;
        }
    };
    if !appended {
        // Already fired (this process on an earlier tick, or a prior
        // supervisor before a restart). At-most-once: do nothing.
        return;
    }

    spawn_hook(cmd, run_id, status_str, &summary, kind, title);
}

/// Spawn `sh -c <cmd>` detached with the completion context in its
/// environment. Not waited on — a hung hook cannot stall the supervisor.
fn spawn_hook(cmd: &str, run_id: &str, status: &str, summary: &str, kind: &str, title: &str) {
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .env("OCTL_RUN_ID", run_id)
        .env("OCTL_STATUS", status)
        .env("OCTL_SUMMARY", summary)
        .env("OCTL_RUN_KIND", kind)
        .env("OCTL_RUN_TITLE", title)
        // Detach stdin; let stdout/stderr inherit so a `notify-send` or a
        // file-append is visible in the supervisor's captured stderr log if it
        // writes there. The hook owns its own output routing otherwise.
        .stdin(std::process::Stdio::null());
    match command.spawn() {
        Ok(_child) => {
            // Fire-and-forget: the grandchild is reparented to init when the
            // supervisor exits moments later, so we do not reap it here (that
            // would reintroduce the blocking wait we are avoiding).
            info!(
                target: "orchestratectl::supervise",
                run_id = %run_id,
                status = %status,
                "fired run completion notify hook"
            );
        }
        Err(e) => {
            // The marker is already durably recorded, so we will NOT retry —
            // at-most-once holds. Surface the spawn failure loudly.
            warn!(
                target: "orchestratectl::supervise",
                run_id = %run_id,
                error = %e,
                "run completion notify hook failed to spawn (not retried; marker already recorded)"
            );
        }
    }
}

/// Read the primary node's terminal `node.report` summary under the shared
/// lock. `None` when there is no node / no report / no `summary` field.
fn read_summary(paths: &RunPaths) -> Option<String> {
    let node_id = NodeId::parse_str(DEFAULT_NODE_ID).expect("DEFAULT_NODE_ID is a valid node id");
    RunLock::with_shared_lock(&paths.lock(), || {
        Ok(read_node_opt(paths, &node_id)?.and_then(|n| n.last_report))
    })
    .map_err(from_core)
    .ok()
    .flatten()
    .as_ref()
    .and_then(|r: &Value| r.get("summary"))
    .and_then(Value::as_str)
    .map(str::to_string)
}

/// Terminal status → the kebab wire string the hook receives in `OCTL_STATUS`.
/// Only terminal statuses reach here; a non-terminal value is defensively
/// rendered rather than panicking.
fn status_kebab(status: Status) -> &'static str {
    match status {
        Status::Done => "done",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Blocked => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::append_and_apply_event;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    const RID: &str = "01jxwd0000000000000000000w";

    /// Build a terminal single-node run with a `node.report` summary. Returns
    /// the run's `RunPaths`. The manifest ends up `done`.
    fn terminal_run(tmp: &TempDir, summary: &str) -> RunPaths {
        let dir = tmp.path().join(RID);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, RID).unwrap();
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.created",
            Some(&NodeId::parse_str(DEFAULT_NODE_ID).unwrap()),
            None,
            json!({ "kind": "spinoff" }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&NodeId::parse_str(DEFAULT_NODE_ID).unwrap()),
            None,
            json!({
                "success": true,
                "failed": false,
                "cancelled": false,
                "summary": summary,
                "discussion_items": [],
                "spinoff_proposals": [],
                "wrap_up_recommendations": [],
            }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "done" }),
        )
        .unwrap();
        paths
    }

    /// Count `run.notified` events in the log.
    fn notified_count(paths: &RunPaths) -> usize {
        std::fs::read_to_string(paths.events())
            .unwrap()
            .lines()
            .filter(|l| l.contains("\"run.notified\""))
            .count()
    }

    /// Poll for `path` to exist (the detached hook is async), up to ~3s.
    fn wait_for_file(path: &std::path::Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        path.exists()
    }

    #[test]
    fn fires_hook_with_completion_env() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "did the thing");
        let out = tmp.path().join("hook-out.txt");
        // The hook records the env the supervisor handed it.
        let cmd = format!(
            "printf '%s|%s|%s|%s' \"$OCTL_RUN_ID\" \"$OCTL_STATUS\" \"$OCTL_SUMMARY\" \"$OCTL_RUN_KIND\" > {}",
            out.display()
        );

        maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t");

        assert!(
            wait_for_file(&out),
            "hook must run and write its output file"
        );
        let got = std::fs::read_to_string(&out).unwrap();
        assert_eq!(got, format!("{RID}|done|did the thing|spinoff"));
        assert_eq!(notified_count(&paths), 1, "exactly one marker recorded");
    }

    #[test]
    fn at_most_once_across_repeated_ticks() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "s");
        let counter = tmp.path().join("counter.txt");
        // Each invocation appends a byte; a second fire would make it 2 bytes.
        let cmd = format!("printf 'x' >> {}", counter.display());

        // First tick fires; a durable marker is recorded.
        maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t");
        assert!(wait_for_file(&counter));
        assert_eq!(notified_count(&paths), 1);

        // Subsequent ticks (and a simulated supervisor restart) must NOT re-fire.
        maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t");
        maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t");

        // Give any (erroneously-spawned) second hook time to land before asserting.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(notified_count(&paths), 1, "marker appended at most once");
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap(),
            "x",
            "the hook ran exactly once despite repeated ticks"
        );
    }

    #[test]
    fn no_notify_cmd_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "s");
        maybe_fire(&paths, RID, None, Status::Done, "spinoff", "t");
        assert_eq!(
            notified_count(&paths),
            0,
            "a run with no --notify records no marker and runs nothing"
        );
    }

    #[test]
    fn read_summary_returns_report_summary() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "the summary line");
        assert_eq!(read_summary(&paths).as_deref(), Some("the summary line"));
    }
}
