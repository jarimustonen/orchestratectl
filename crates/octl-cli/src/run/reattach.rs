//! `run reattach` — restart the supervisor for an existing run.
//!
//! Refuses if `<run-dir>/supervisor.pid` is still alive (use `run
//! cancel` or kill the existing supervisor instead). Otherwise: emits
//! `supervisor.reattached`, fork+exec a new `orchestratectl supervise
//! <run-id>` with stdout/stderr redirected to
//! `<run-dir>/supervisor.stderr.log`, and waits briefly for the new
//! supervisor's PID file to appear.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use octl_core::{append_and_apply, read_manifest_opt};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, require_safe_id, run_paths};
use crate::supervise::pid_file;

const PID_FILE_WAIT: Duration = Duration::from_secs(5);

#[derive(Serialize)]
struct ReattachPayload<'a> {
    run_id: &'a str,
    action: &'static str,
    supervisor_pid: u32,
}

pub fn run(
    run_id: &str,
    once: bool,
    max_iter: Option<u32>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let run_id = require_safe_id(run_id, "run-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }
    let pid_path = paths.supervisor_pid();
    if let Some(existing) = pid_file::read_pid(&pid_path) {
        if pid_file::pid_alive(existing) {
            return Err(CliError::system(
                "supervisor_already_running",
                format!("supervisor pid {existing} for run {run_id} is alive (no reattach needed)"),
            ));
        }
        // Stale PID file: record the dead prior incarnation.
        let _ = append_and_apply(
            &paths,
            "supervisor.exited",
            None,
            None,
            json!({"pid": existing, "reason": "stale-on-reattach"}),
        );
    }

    // Record the request, then spawn.
    append_and_apply(
        &paths,
        "supervisor.reattach-requested",
        None,
        None,
        json!({}),
    )
    .map_err(from_core)?;

    let stderr_path: PathBuf = paths.root.join("supervisor.stderr.log");
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
    let mut cmd = Command::new(exe);
    cmd.arg("supervise").arg(&run_id);
    if once {
        cmd.arg("--once");
    }
    if let Some(n) = max_iter {
        cmd.arg("--max-iter").arg(n.to_string());
    }
    let child = cmd
        .stdout(stderr_file)
        .stderr(stderr_clone)
        .spawn()
        .map_err(|e| {
            CliError::system("spawn_failed", format!("spawn supervise {}: {}", run_id, e))
        })?;
    let child_pid = child.id();

    // Wait briefly for the child to write its own PID file. We may see
    // either `child_pid` (the new supervisor we just spawned) or — if a
    // human or test reattach raced us — a different PID; either way the
    // contract is that *some* live supervisor now owns the run.
    let deadline = Instant::now() + PID_FILE_WAIT;
    let recorded_pid = loop {
        if let Some(p) = pid_file::read_pid(&pid_path) {
            if pid_file::pid_alive(p) {
                break p;
            }
        }
        if Instant::now() >= deadline {
            // Best-effort fallback: report the PID we spawned even if
            // the file hasn't landed yet (it should within a tick).
            break child_pid;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let _ = append_and_apply(
        &paths,
        "supervisor.reattached",
        None,
        None,
        json!({"pid": recorded_pid}),
    );

    let payload = ReattachPayload {
        run_id: &run_id,
        action: "reattached",
        supervisor_pid: recorded_pid,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!(
                "reattached run {} (supervisor pid {})",
                run_id, recorded_pid
            );
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
