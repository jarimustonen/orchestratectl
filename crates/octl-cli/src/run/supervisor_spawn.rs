//! Spawn a detached `orchestratectl supervise <run-id>` process and
//! wait briefly for its PID file to appear.
//!
//! Shared by `run create` (top-level), `run reattach`, and the parent
//! supervisor's child-spawn loop. All three funnel through the same
//! [`detached_supervise_command`] + [`spawn_and_reap`] pair so the process
//! detachment hardening (the double-fork below) lives in exactly one place.
//!
//! # Why double-fork
//!
//! A supervisor must outlive both the terminal that launched it and the
//! process that spawned it, and must never linger as a zombie. The naive
//! `Command::spawn` leaves the child in the spawner's session/process group
//! (so closing the terminal `SIGHUP`s every supervisor) and as a direct
//! child (so an exited supervisor becomes a zombie until the spawner
//! `wait()`s it — and `kill(pid, 0)` reports a zombie as *alive*, corrupting
//! the PID-staleness check).
//!
//! The fix is a classic double-fork, run inside `pre_exec` (after `fork`,
//! before `exec`):
//!   1. `setsid()` — the child leads a new session with no controlling
//!      terminal, so a terminal `SIGHUP` can never reach it.
//!   2. `fork()` again — the intermediate exits *immediately* (`_exit`,
//!      bypassing Rust destructors), so the grandchild is orphaned and
//!      reparented to init (pid 1). Init reaps the grandchild when it
//!      eventually exits, so no zombie ever accrues on our side.
//!
//! The spawner then `wait()`s the short-lived intermediate (it has already
//! `_exit`ed, so the wait returns at once) to reap *it*. Net: nothing the
//! spawner owns can become a zombie, and the real supervisor is fully
//! detached.
//!
//! Because this runs in `pre_exec` — before the supervisor binary's
//! `main`/tracing-subscriber ever initialize — there is no worker thread to
//! be orphaned by the intermediate's exit (a real hazard if you double-fork
//! *after* the runtime starts). The grandchild builds its tracing stack
//! fresh after `exec`.
//!
//! One consequence: the PID `Command::spawn` returns is the intermediate's,
//! not the grandchild's. The authoritative supervisor PID is the one the
//! supervisor writes into its own `supervisor.pid` during
//! [`crate::supervise::pid_file::claim_pid_atomic`]; callers that need it
//! read it back from that file (see [`spawn_for_run`]).

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use octl_core::RunPaths;

use crate::error::CliError;
use crate::supervise::pid_file;

const PID_FILE_WAIT: Duration = Duration::from_secs(5);
const POLL_TICK: Duration = Duration::from_millis(200);

/// Outcome of a supervisor spawn: the PID we recorded on the run.
pub struct SupervisorSpawn {
    pub pid: u32,
}

/// Attach the detach hardening (`setsid` + double-fork) to `cmd`'s child via
/// `pre_exec`. See the module docs for the full rationale.
fn apply_detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure is async-signal-safe — it calls only `setsid`,
    // `fork`, and `_exit` (all on the POSIX async-signal-safe list) and does
    // no allocation, locking, or other Rust-runtime work between `fork` and
    // `exec`.
    unsafe {
        cmd.pre_exec(|| {
            // New session: detach from the controlling terminal and the
            // spawner's process group so a terminal SIGHUP cannot reach us.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Double-fork: the intermediate exits now so the grandchild is
            // reparented to init (pid 1), which reaps it on exit. Use `_exit`
            // (not `return`/`exit`) so no Rust destructors or atexit hooks run
            // in this forked-but-not-exec'd process.
            match libc::fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => Ok(()),         // grandchild: continue to exec the supervisor
                _ => libc::_exit(0), // intermediate: vanish, orphaning the grandchild
            }
        });
    }
}

/// Build a detach-hardened `supervise <run-id>` command with stdout/stderr
/// redirected to `log_path`. Callers may append extra args (e.g. `--once`)
/// before handing it to [`spawn_and_reap`].
pub fn detached_supervise_command(run_id: &str, log_path: &Path) -> Result<Command, CliError> {
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| CliError::system("io_error", format!("open {}: {}", log_path.display(), e)))?;
    let stderr_clone = stderr_file
        .try_clone()
        .map_err(|e| CliError::system("io_error", format!("dup fd: {e}")))?;
    let exe = std::env::current_exe()
        .map_err(|e| CliError::system("io_error", format!("current_exe: {e}")))?;
    let mut cmd = Command::new(exe);
    cmd.arg("supervise")
        .arg(run_id)
        // A detached daemon must not keep the launching terminal's stdin: an
        // inherited TTY fd 0 can deliver SIGTTIN or let a stray read consume
        // terminal input. setsid drops the controlling-terminal relationship
        // but not the fd itself.
        .stdin(std::process::Stdio::null())
        .stdout(stderr_file)
        .stderr(stderr_clone);
    apply_detach(&mut cmd);
    Ok(cmd)
}

/// Spawn a detach-hardened supervisor `cmd` and reap the short-lived
/// double-fork intermediate so it never lingers as a zombie. The real
/// supervisor (the grandchild) is already reparented to init by the time
/// this returns.
pub fn spawn_and_reap(cmd: &mut Command, run_id: &str) -> Result<(), CliError> {
    let mut child = cmd
        .spawn()
        .map_err(|e| CliError::system("spawn_failed", format!("spawn supervise {run_id}: {e}")))?;
    // The intermediate `_exit`s immediately after forking the grandchild, so
    // this wait reaps it without blocking on the actual supervisor. A wait
    // error (e.g. ECHILD if it was already reaped) is non-fatal — the
    // grandchild is independent — but surface it rather than swallow it.
    if let Err(e) = child.wait() {
        tracing::warn!(
            target: "orchestratectl::supervise",
            run = %run_id,
            error = %e,
            "failed to reap double-fork intermediate (grandchild unaffected)"
        );
    }
    Ok(())
}

/// Read `<run-dir>/supervisor.pid` ONCE and return the recorded pid iff it is
/// a live process whose start-time still matches the record (§7.6 identity
/// check). Non-blocking. Used where the caller must not stall — e.g. the
/// parent supervisor's tick — and is content with "pid not yet confirmed"
/// (the child writes its own pid file as the durable source of truth).
pub fn read_live_recorded_pid(paths: &RunPaths) -> Option<u32> {
    let (pid, start_time) = pid_file::read_pid_record(&paths.supervisor_pid())?;
    pid_file::pid_live_with_identity(pid, start_time).then_some(pid)
}

/// Poll `<run-dir>/supervisor.pid` for up to [`PID_FILE_WAIT`] and return the
/// live, identity-verified supervisor PID it records. `None` if none appears
/// in time — with double-fork we have no usable spawned PID to fall back to
/// (the intermediate we reaped is gone), so callers decide how to degrade. In
/// practice the supervisor writes its pid file under the run flock within
/// milliseconds of `exec`, so the deadline is reached only if the supervisor
/// failed to boot.
///
/// Identity matters: a stale pid file from a prior generation whose pid has
/// been recycled by an unrelated live process must NOT be accepted as "our"
/// supervisor — hence `read_pid_record` + `pid_live_with_identity`, not a bare
/// liveness probe.
pub fn await_recorded_pid(paths: &RunPaths) -> Option<u32> {
    let deadline = Instant::now() + PID_FILE_WAIT;
    loop {
        if let Some(pid) = read_live_recorded_pid(paths) {
            return Some(pid);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL_TICK);
    }
}

/// Fork+exec a fully-detached supervisor with stdout/stderr redirected to
/// `<run-dir>/supervisor.stderr.log`, then wait up to 5s for the
/// supervisor's own PID file to appear and be alive.
///
/// Returns the PID the supervisor recorded for itself. On timeout (the
/// supervisor did not write a live pid file) returns `pid: 0` — a sentinel
/// the caller surfaces as "spawned but PID not yet confirmed"; the
/// supervisor's own `supervisor.pid` remains the source of truth.
pub fn spawn_for_run(paths: &RunPaths, run_id: &str) -> Result<SupervisorSpawn, CliError> {
    let log_path = paths.root.join("supervisor.stderr.log");
    let mut cmd = detached_supervise_command(run_id, &log_path)?;
    spawn_and_reap(&mut cmd, run_id)?;
    let pid = await_recorded_pid(paths).unwrap_or(0);
    Ok(SupervisorSpawn { pid })
}
