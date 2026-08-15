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

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::Command;

use clap::Args as ClapArgs;
use serde_json::{json, Value};

use octl_core::{append_and_apply_event, read_manifest_opt, read_node_opt, NodeId, RunPaths};

use crate::error::CliError;
use crate::run::{from_core, parse_node_id, parse_run_id, run_paths_exact};

/// Signals the shim IGNORES while it waits on the child, restoring their default
/// disposition in the child via `pre_exec` (below). A worker launched into an
/// interactive PTY shares the foreground process group, so a `SIGINT` (Ctrl-C) —
/// or a `SIGHUP`/`SIGTERM` from a pane/window teardown — is delivered to the shim
/// as well as the child. If the shim died from it before recording, the told-fact
/// guarantee would collapse to the pid-guess it exists to replace. Ignoring these
/// in the shim (while the child keeps its default disposition) lets the child take
/// the signal, die, and be reaped so the shim records its TRUE status and exits
/// with it. `SIGKILL` cannot be caught — that is the residual case the crash
/// backstop (design.md §2.1a) covers.
const SHIM_IGNORED_SIGNALS: [libc::c_int; 4] =
    [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// Synthetic exit code recorded when the worker could not be *launched* at all
/// (e.g. the program is missing / not executable). Mirrors the shell's
/// "command not found" convention so the told fact is a plain non-zero failure —
/// the supervisor terminalizes `failed` (branch preserved) instead of falling
/// back to the pid-guess this shim exists to eliminate.
const SPAWN_FAILURE_EXIT_CODE: i32 = 127;

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
    // Validate ids and resolve the run + node BEFORE spawning: a typo must fail
    // loudly here rather than launch a worker whose exit the reducer would fold to
    // nothing (a `worker.exited` for an unknown node is a silent no-op).
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
    if read_node_opt(&paths, &node_id)
        .map_err(from_core)?
        .is_none()
    {
        return Err(CliError::user(
            "node_not_found",
            format!("no node {} in run {}", args.node_id, args.run_id),
        )
        .with_invalid_value(&args.node_id));
    }

    // `required = true` guarantees at least the program; split off its args.
    let (program, prog_args) = args
        .command
        .split_first()
        .ok_or_else(|| CliError::user("missing_worker_command", "no worker command after `--`"))?;

    // Survive foreground-group signals so the child owns them and the shim lives
    // long enough to record the child's true status (see `SHIM_IGNORED_SIGNALS`).
    // SAFETY: `libc::signal` is async-signal-safe and this runs single-threaded at
    // startup before the child is spawned.
    for sig in SHIM_IGNORED_SIGNALS {
        unsafe { libc::signal(sig, libc::SIG_IGN) };
    }

    // Inherit stdio (the default): the wrapped agent runs its interactive TUI in
    // this pane's PTY. No timeout — the worker owns its own lifecycle; the shim
    // only observes the exit. The child restores DEFAULT signal dispositions (via
    // `pre_exec`, after fork / before exec) so it still takes a Ctrl-C / teardown
    // signal normally even though the shim ignores it.
    let mut cmd = Command::new(program);
    cmd.args(prog_args);
    // SAFETY: the closure only calls the async-signal-safe `libc::signal` in the
    // forked child before `exec`; it touches no shared state and allocates nothing.
    unsafe {
        cmd.pre_exec(|| {
            for sig in SHIM_IGNORED_SIGNALS {
                libc::signal(sig, libc::SIG_DFL);
            }
            Ok(())
        });
    }

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            // The worker could not even be launched. Record a told FAILURE (rather
            // than let the supervisor fall back to pid-guessing) and then surface
            // the error normally.
            let mut data = serde_json::Map::new();
            data.insert("exit_code".into(), json!(SPAWN_FAILURE_EXIT_CODE));
            record_worker_exit(&paths, &node_id, &args, Value::Object(data));
            return Err(CliError::system(
                "worker_spawn_failed",
                format!("could not launch worker `{program}`: {e}"),
            ));
        }
    };

    // Exactly one of code / signal is meaningful on Unix: a normal return carries
    // a code (signal None); a signal death carries a signal (code None). Prefer the
    // signal when present so the fact is never the ambiguous both-fields shape the
    // reducer rejects.
    let exit_code = status.code();
    let signal = status.signal();
    let mut data = serde_json::Map::new();
    match (signal, exit_code) {
        (Some(s), _) => {
            data.insert("signal".into(), json!(s));
        }
        (None, Some(c)) => {
            data.insert("exit_code".into(), json!(c));
        }
        // Neither present is unreachable on Unix (ExitStatus always carries one),
        // but the reducer rejects an empty payload, so record a synthetic
        // non-zero code rather than lose the fact.
        (None, None) => {
            data.insert("exit_code".into(), json!(-1));
        }
    }

    record_worker_exit(&paths, &node_id, &args, Value::Object(data));

    // Propagate the worker's own exit status. `process::exit` runs no
    // destructors, so flush the buffered tracing log first (parity with the
    // supervisor's signal-exit path).
    let code = exit_code.unwrap_or_else(|| 128 + signal.unwrap_or(1));
    crate::cli::flush_logs();
    std::process::exit(code);
}

/// Append the durable `worker.exited` fact under the run lock (design.md §2.1).
///
/// No idempotency key: the shim fires once per launched worker, and the reducer's
/// first-write-wins guard already dedups a replay. A KEY scoped to the node would
/// silently swallow a *retried* worker's exit (a second shim for the same node),
/// so it is deliberately absent — attempt-scoped recording is left to the
/// attempt-identity work when retries are wired through the shim.
///
/// A recording failure does not swallow the worker's status (the crash backstop,
/// design.md §2.1a, covers a missing exit event), but it MUST be visible: it is
/// surfaced on stderr (the pane the operator sees) as well as the tracing log.
fn record_worker_exit(paths: &RunPaths, node_id: &NodeId, args: &RunWorkerArgs, data: Value) {
    if let Err(e) = append_and_apply_event(paths, "worker.exited", Some(node_id), None, data) {
        let msg = from_core(e).message;
        eprintln!(
            "orchestratectl run-worker: failed to record worker.exited for {}/{}: {msg}",
            args.run_id, args.node_id
        );
        tracing::warn!(
            target: "orchestratectl::run_worker",
            run_id = %args.run_id,
            node_id = %args.node_id,
            error = %msg,
            "failed to record worker.exited event; relying on the crash backstop"
        );
    }
}
