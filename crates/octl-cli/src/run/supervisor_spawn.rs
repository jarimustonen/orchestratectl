//! Spawn a detached `orchestratectl supervise <run-id>` process and
//! wait briefly for its PID file to appear.
//!
//! Shared by `run create` (top-level) and `run reattach`. Child-spawn
//! does NOT use this helper — design.md §7.2 step 6 reserves child
//! supervisor spawn for the *parent* supervisor's tail-follow loop, so
//! that exact-once supervisor spawn is the parent's invariant.

use std::path::PathBuf;
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

/// Fork+exec the supervisor with stdout/stderr redirected to
/// `<run-dir>/supervisor.stderr.log`, then wait up to 5s for the
/// supervisor's own PID file to appear and be alive.
///
/// Falls back to the spawned process's PID if the file hasn't landed
/// within the deadline — the supervisor will overwrite it on its first
/// tick, and the recorded PID is still the right one to report.
pub fn spawn_for_run(paths: &RunPaths, run_id: &str) -> Result<SupervisorSpawn, CliError> {
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
    let child = Command::new(exe)
        .arg("supervise")
        .arg(run_id)
        .stdout(stderr_file)
        .stderr(stderr_clone)
        .spawn()
        .map_err(|e| {
            CliError::system("spawn_failed", format!("spawn supervise {}: {}", run_id, e))
        })?;
    let spawned_pid = child.id();

    let pid_path = paths.supervisor_pid();
    let deadline = Instant::now() + PID_FILE_WAIT;
    let recorded_pid = loop {
        if let Some(p) = pid_file::read_pid(&pid_path) {
            if pid_file::pid_alive(p) {
                break p;
            }
        }
        if Instant::now() >= deadline {
            break spawned_pid;
        }
        std::thread::sleep(POLL_TICK);
    };

    Ok(SupervisorSpawn { pid: recorded_pid })
}
