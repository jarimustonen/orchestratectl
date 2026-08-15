//! `orchestratectl run-worker <run-id> <node-id> -- <cmd> [args…]` — the thin
//! launcher shim (design.md §2.1 / A1, issue `thin-exit-status-launcher`).
//!
//! The 0.2 thin supervisor stops *guessing* a worker's completion from a
//! cross-product of pid × pane × activity proxies and instead consumes **told
//! facts**. This shim supplies the first such fact: an autonomous worker is
//! launched *through* it (`run-worker <run> <node> -- pi …`), so the shim
//! `wait()`s on the real agent process and records its **true exit status** —
//! a normal `exit_code` or a terminating `signal` — as a durable `worker.exited`
//! event under the run lock. The supervisor then reads a recorded status, not a
//! liveness inference (see [`crate::supervise`] exit-status consumption):
//!
//! - non-zero exit / killed by signal → `failed` (branch preserved),
//! - exit 0 **and** an `explicit-merge` transition exists → `done` + teardown,
//! - exit 0 **and** no merge → stays non-terminal / attention-required (the
//!   finished-but-unmerged case handed to the manual finish skill), NOT
//!   auto-failed.
//!
//! This is a *shim, not a protocol*: it needs no per-SKILL churn — the worker
//! command is simply wrapped at launch. It inherits stdio so the wrapped agent's
//! interactive TUI shares the pane exactly as an un-wrapped launch would, and it
//! never times the child out — the agent owns its own runtime.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use clap::Args as ClapArgs;
use serde_json::{json, Value};

use octl_core::{append_and_apply_event, read_manifest_opt};

use crate::error::CliError;
use crate::run::{from_core, parse_node_id, parse_run_id, run_paths_exact};

#[derive(ClapArgs, Debug)]
pub struct RunWorkerArgs {
    /// Run id the worker belongs to (full ULID).
    pub run_id: String,
    /// Node id inside the run whose worker this is (e.g. `n-0001`).
    pub node_id: String,
    /// The worker command and its arguments, given after `--`
    /// (e.g. `run-worker <run> <node> -- pi --task …`).
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

/// Launch the wrapped worker, wait for it, record its exit status durably, and
/// exit with the worker's own status. Never returns on the success path — it
/// `process::exit`s with the child's code (or `128 + signal`, the shell
/// convention) so the pane's exit status mirrors the agent's.
pub fn dispatch(args: RunWorkerArgs) -> Result<(), CliError> {
    // Validate ids and resolve the run BEFORE spawning: a typo must fail loudly
    // here rather than launch a worker whose exit can never be attributed.
    let run_id = parse_run_id(&args.run_id)?;
    let node_id = parse_node_id(&args.node_id)?;
    let root = crate::home::root_dir()?;
    let paths = run_paths_exact(&root, &run_id)?;
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {}", args.run_id))
                .with_invalid_value(&args.run_id),
        );
    }

    // `required = true` guarantees at least the program; split off its args.
    let (program, prog_args) = args
        .command
        .split_first()
        .ok_or_else(|| CliError::user("missing_worker_command", "no worker command after `--`"))?;

    // Inherit stdio (the default): the wrapped agent runs its interactive TUI in
    // this pane's PTY. No timeout — the worker owns its own lifecycle; the shim
    // only observes the exit.
    let status = Command::new(program)
        .args(prog_args)
        .status()
        .map_err(|e| {
            CliError::system(
                "worker_spawn_failed",
                format!("could not launch worker `{program}`: {e}"),
            )
        })?;

    // Exactly one of code / signal is meaningful on Unix: a normal return
    // carries a code (signal None); a signal death carries a signal (code None).
    let exit_code = status.code();
    let signal = status.signal();

    let mut data = serde_json::Map::new();
    match (exit_code, signal) {
        (Some(c), _) => {
            data.insert("exit_code".into(), json!(c));
            if let Some(s) = signal {
                // Defensive: a platform reporting both — record both facts.
                data.insert("signal".into(), json!(s));
            }
        }
        (None, Some(s)) => {
            data.insert("signal".into(), json!(s));
        }
        // Neither present is unreachable on Unix (ExitStatus always carries one),
        // but the reducer rejects an empty payload, so record a synthetic
        // non-zero code rather than lose the fact.
        (None, None) => {
            data.insert("exit_code".into(), json!(-1));
        }
    }

    // Record the told fact durably under the run lock. Idempotency-keyed so a
    // (pathological) re-invocation for the same node dedups to the first exit;
    // `append_and_apply_event` acquires the flock itself, so the invariant-5
    // `LockedRun` witness is threaded internally.
    let key = format!("worker-exit:{node_id}");
    if let Err(e) = append_and_apply_event(
        &paths,
        "worker.exited",
        Some(&node_id),
        Some(&key),
        Value::Object(data),
    ) {
        // Do NOT swallow the worker's status over a recording failure: the crash
        // backstop (§2.1a) covers a missing exit event. Warn and still propagate
        // the child's code so the pane reflects reality.
        tracing::warn!(
            target: "orchestratectl::run_worker",
            run_id = %args.run_id,
            node_id = %args.node_id,
            error = %from_core(e).message,
            "failed to record worker.exited event; relying on the crash backstop"
        );
    }

    // Propagate the worker's own exit status. `process::exit` runs no
    // destructors, so flush the buffered tracing log first (parity with the
    // supervisor's signal-exit path).
    let code = exit_code.unwrap_or_else(|| 128 + signal.unwrap_or(1));
    crate::cli::flush_logs();
    std::process::exit(code);
}
