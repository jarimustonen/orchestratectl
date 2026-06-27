//! `run reattach` — restart the supervisor for an existing run.
//!
//! Refuses if `<run-dir>/supervisor.pid` is still alive (use `run
//! cancel` or kill the existing supervisor instead). Otherwise: emits
//! `supervisor.reattached`, fork+exec a new `orchestratectl supervise
//! <run-id>` with stdout/stderr redirected to
//! `<run-dir>/supervisor.stderr.log`, and waits briefly for the new
//! supervisor's PID file to appear.

use serde::Serialize;
use serde_json::json;

use octl_core::{append_and_apply_event, read_manifest_opt};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::supervisor_spawn;
use crate::run::{from_core, run_paths};
use crate::supervise::pid_file;

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
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id)?;
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id),
        );
    }
    let pid_path = paths.supervisor_pid();
    if let Some((existing, start_time)) = pid_file::read_pid_record(&pid_path) {
        if pid_file::pid_live_with_identity(existing, start_time) {
            return Err(CliError::system(
                "supervisor_already_running",
                format!("supervisor pid {existing} for run {run_id} is alive (no reattach needed)"),
            ));
        }
        // Stale PID file (dead or recycled PID): record the dead prior
        // incarnation.
        let _ = append_and_apply_event(
            &paths,
            "supervisor.exited",
            None,
            None,
            json!({"pid": existing, "reason": "stale-on-reattach"}),
        );
    }

    // Record the request, then spawn.
    append_and_apply_event(
        &paths,
        "supervisor.reattach-requested",
        None,
        None,
        json!({}),
    )
    .map_err(from_core)?;

    // Fork+exec a fully-detached supervisor (setsid + double-fork; see
    // `supervisor_spawn`). The spawned supervisor performs the atomic
    // `claim_pid_atomic` in its own startup, so even if two reattaches race
    // past the stale-pid pre-check above, exactly one spawned supervisor
    // wins the flock-guarded claim and the loser exits.
    let log_path = paths.root.join("supervisor.stderr.log");
    let mut cmd = supervisor_spawn::detached_supervise_command(run_id, &log_path)?;
    if once {
        cmd.arg("--once");
    }
    if let Some(n) = max_iter {
        cmd.arg("--max-iter").arg(n.to_string());
    }
    supervisor_spawn::spawn_and_reap(&mut cmd, run_id)?;

    // Wait briefly for the new supervisor to write its own PID file. We may
    // see the PID it claimed or — if a human or test reattach raced us — a
    // different one; either way the contract is that *some* live supervisor
    // now owns the run. With the double-fork we have no usable spawned PID to
    // report on timeout, so fall back to whatever the file last recorded.
    let recorded_pid = supervisor_spawn::await_recorded_pid(&paths)
        .or_else(|| pid_file::read_pid(&pid_path))
        .unwrap_or(0);

    let _ = append_and_apply_event(
        &paths,
        "supervisor.reattached",
        None,
        None,
        json!({"pid": recorded_pid}),
    );

    let payload = ReattachPayload {
        run_id,
        action: "reattached",
        supervisor_pid: recorded_pid,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("reattached run {run_id} (supervisor pid {recorded_pid})");
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
