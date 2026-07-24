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
//! The command runs via `sh -c` and the supervisor never `wait`s on it inline,
//! so a slow or hung hook can never wedge the single-threaded supervisor tick.
//! A dedicated reaper thread blocks on the child so a fast hook is collected
//! promptly rather than lingering as a zombie until the supervisor exits (which
//! is unbounded for a cancelled parent that keeps ticking until its children
//! settle).

use serde_json::{json, Value};
use tracing::{info, warn};

use octl_core::{read_node_opt, NodeId, RunLock, RunPaths, Status};

use crate::run::from_core;

/// Reporting node whose terminal `node.report` carries the run's outcome
/// summary. Every single-worker worktree kind has exactly one node
/// (`n-0001`); mirrors `run wait`'s and `run merge`'s `DEFAULT_NODE_ID`.
const DEFAULT_NODE_ID: &str = "n-0001";

/// Cap on the `OCTL_SUMMARY` env value (bytes-ish, counted in chars). A
/// `node.report` summary is arbitrary agent-authored text; an unbounded value
/// risks `E2BIG` at `spawn` time, which — because the durable marker is already
/// recorded — would permanently drop the notification. Bounded well under the
/// platform `ARG_MAX`/env ceiling with headroom for the rest of the environment.
const SUMMARY_MAX_CHARS: usize = 4096;
/// Cap on the `OCTL_RUN_TITLE` env value; a title is short by construction, but
/// bound it defensively for the same reason.
const TITLE_MAX_CHARS: usize = 512;

/// Fire the run's `--notify` hook once, if one is registered and this is the
/// first time the run is observed terminal.
///
/// `status` is the run's terminal manifest status; `kind`/`title` are surfaced
/// to the hook for a richer message. Best-effort throughout: any failure is
/// logged and swallowed — a broken notification must never block teardown or
/// crash the supervisor.
///
/// # Return
///
/// `true` when there is nothing left to do for this run — no hook was
/// registered, the hook was already fired (by this process on an earlier tick
/// or by a pre-restart supervisor), or the hook was spawned just now. `false`
/// only when recording the durable marker failed transiently (lock contention,
/// I/O): the caller should keep the run's `notified` flag unset so a later tick
/// retries, honestly delivering on the "retry" contract that the old
/// unconditional-`cleaned` gate silently broke.
#[must_use]
pub fn maybe_fire(
    paths: &RunPaths,
    run_id: &str,
    notify_cmd: Option<&str>,
    status: Status,
    kind: &str,
    title: &str,
) -> bool {
    let Some(cmd) = notify_cmd else {
        return true;
    };
    // Defensive: the marker is a permanent gate, so never fire (and never
    // record it) for a non-terminal status. The sole caller guards on
    // `status.is_terminal()`, but a future caller that forgets to must not be
    // able to poison the gate on a still-running run.
    if !status.is_terminal() {
        return true;
    }

    let summary = env_safe(&read_summary(paths).unwrap_or_default(), SUMMARY_MAX_CHARS);
    let title = env_safe(title, TITLE_MAX_CHARS);
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
        json!({ "status": status_str }),
    ) {
        Ok(res) => !res.idempotent_replay,
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                run_id = %run_id,
                error = %e,
                "could not record run.notified marker; will retry on a later tick"
            );
            return false;
        }
    };
    if !appended {
        // Already fired (this process on an earlier tick, or a prior
        // supervisor before a restart). At-most-once: do nothing.
        return true;
    }

    spawn_hook(cmd, run_id, status_str, &summary, kind, &title);
    true
}

/// Spawn `sh -c <cmd>` with the completion context in its environment, and
/// reap it on a detached thread so a fast hook never lingers as a zombie.
///
/// The supervisor is single-threaded and must not block on the hook — a hung
/// command cannot stall the tick — so we do not `wait` inline. But a plain
/// `spawn` + drop of the `Child` does NOT reap: the finished `sh` would sit as
/// a zombie until the supervisor process exits, which is unbounded for a
/// cancelled parent that keeps ticking until its children settle. A dedicated
/// thread that blocks on `child.wait()` reaps it promptly while blocking only
/// itself. Stdout/stderr go to `/dev/null`: the supervisor's own stderr fd can
/// be closed out from under a still-running detached hook (→ `EBADF`/`SIGPIPE`
/// mid-write), and a hook that wants output routes it itself (`>> file 2>&1`).
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
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match command.spawn() {
        Ok(child) => {
            // Reap asynchronously: the thread outlives the tick but not the
            // hook, and never blocks the supervisor loop.
            std::thread::spawn(move || {
                let mut child = child;
                let _ = child.wait();
            });
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

/// Sanitize a string for use as a process environment value: drop NUL bytes
/// (which `Command::env` rejects — a NUL in an agent-authored summary would
/// otherwise fail `spawn` *after* the durable marker, silently dropping the
/// notification) and bound the length so an oversized value can't trip
/// `E2BIG`. Truncation is on a char boundary with an ellipsis marker.
fn env_safe(s: &str, max_chars: usize) -> String {
    let filtered: String = s.chars().filter(|&c| c != '\0').collect();
    if filtered.chars().count() > max_chars {
        let mut out: String = filtered.chars().take(max_chars).collect();
        out.push('…');
        out
    } else {
        filtered
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

    /// Count `run.notified` events in the log, matching the parsed event
    /// `kind` (not a substring, which a data payload could false-match).
    fn notified_count(paths: &RunPaths) -> usize {
        std::fs::read_to_string(paths.events())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v.get("kind").and_then(Value::as_str) == Some("run.notified"))
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

        assert!(
            maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t"),
            "a fired hook settles the notify state"
        );

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
        assert!(maybe_fire(
            &paths,
            RID,
            Some(&cmd),
            Status::Done,
            "spinoff",
            "t"
        ));
        assert!(wait_for_file(&counter));
        assert_eq!(notified_count(&paths), 1);

        // Subsequent ticks (and a simulated supervisor restart) must NOT re-fire,
        // and each reports "settled" so the loop does not spin retrying.
        assert!(maybe_fire(
            &paths,
            RID,
            Some(&cmd),
            Status::Done,
            "spinoff",
            "t"
        ));
        assert!(maybe_fire(
            &paths,
            RID,
            Some(&cmd),
            Status::Done,
            "spinoff",
            "t"
        ));

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
        assert!(
            maybe_fire(&paths, RID, None, Status::Done, "spinoff", "t"),
            "no hook registered is a settled no-op"
        );
        assert_eq!(
            notified_count(&paths),
            0,
            "a run with no --notify records no marker and runs nothing"
        );
    }

    #[test]
    fn non_terminal_status_never_fires_or_marks() {
        // Defensive guard: a non-terminal status must not poison the durable
        // gate — no marker, no hook, and it reports "settled" so a caller does
        // not spin retrying.
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "s");
        let out = tmp.path().join("should-not-exist");
        let cmd = format!("touch {}", out.display());
        assert!(maybe_fire(
            &paths,
            RID,
            Some(&cmd),
            Status::Running,
            "spinoff",
            "t"
        ));
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !out.exists(),
            "hook must not fire for a non-terminal status"
        );
        assert_eq!(
            notified_count(&paths),
            0,
            "no marker for a non-terminal run"
        );
    }

    #[test]
    fn returns_true_on_idempotent_replay() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "s");
        let cmd = "true".to_string();
        assert!(maybe_fire(
            &paths,
            RID,
            Some(&cmd),
            Status::Done,
            "spinoff",
            "t"
        ));
        // Second call sees the marker → idempotent replay → settled, no re-fire.
        assert!(
            maybe_fire(&paths, RID, Some(&cmd), Status::Done, "spinoff", "t"),
            "an already-fired hook reports settled"
        );
        assert_eq!(notified_count(&paths), 1);
    }

    #[test]
    fn env_safe_strips_nul_and_bounds_length() {
        // NUL is stripped (Command::env would otherwise reject the value).
        assert_eq!(env_safe("a\0b\0c", 100), "abc");
        // Under the cap is passed through unchanged.
        assert_eq!(env_safe("short", 100), "short");
        // Over the cap is truncated with an ellipsis marker.
        let long = "x".repeat(50);
        let got = env_safe(&long, 10);
        assert_eq!(got.chars().filter(|&c| c == 'x').count(), 10);
        assert!(got.ends_with('…'));
    }

    #[test]
    fn read_summary_returns_report_summary() {
        let tmp = TempDir::new().unwrap();
        let paths = terminal_run(&tmp, "the summary line");
        assert_eq!(read_summary(&paths).as_deref(), Some("the summary line"));
    }
}
