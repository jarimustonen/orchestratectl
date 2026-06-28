//! Shared integration-test fixtures.
//!
//! [`TestHome`] is a `TempDir`-backed `ORCHESTRATECTL_HOME` that reaps every
//! supervisor process spawned beneath it when it drops, so the test suite
//! never leaks `orchestratectl supervise` processes
//! (issue: supervise-test-teardown-leak).
//!
//! `#![allow(dead_code)]` because each integration-test binary compiles this
//! module independently and uses only the subset it needs.
#![allow(dead_code)]

use std::ops::Deref;
use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Grace period between the polite SIGTERM and the SIGKILL escalation for a
/// supervisor that does not exit promptly.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// A `TempDir` used as `ORCHESTRATECTL_HOME` that reaps the supervisor
/// processes spawned beneath it on drop.
///
/// `run create` (and `run reattach`) spawn a top-level supervisor that
/// double-forks and `setsid`s into its own session, reparenting to init — it
/// is therefore *not* a child of the test process (so `waitpid` cannot reap
/// it) and lives outside the test's own process group (so a harness-wide
/// `killpg` can never reach it). The authoritative handle is the PID each
/// supervisor writes into `<run-dir>/supervisor.pid`; on drop we scan every
/// run dir under the home and reap each still-live supervisor.
///
/// Derefs to the inner [`TempDir`] so existing helpers that take `&TempDir`
/// (and `home.path()`) keep working unchanged. The reap happens in
/// [`Drop::drop`] *before* the inner `TempDir` field is dropped, so the run
/// dirs — and their pid files — still exist when we read them.
pub struct TestHome {
    dir: TempDir,
}

impl TestHome {
    /// Create a fresh temp home. Panics on failure (a test cannot proceed
    /// without an isolated home), mirroring the previous
    /// `TempDir::new().unwrap()` call sites.
    pub fn new() -> Self {
        Self {
            dir: TempDir::new().expect("create temp ORCHESTRATECTL_HOME"),
        }
    }
}

impl Default for TestHome {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for TestHome {
    type Target = TempDir;
    fn deref(&self) -> &TempDir {
        &self.dir
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        reap_supervisors_under(self.dir.path());
        // `self.dir` (the TempDir) drops *after* this body, removing the tree.
    }
}

/// SIGTERM, then — after [`REAP_GRACE`] — SIGKILL every live supervisor whose
/// pid file lives under `<home>/runs/*/supervisor.pid`.
pub fn reap_supervisors_under(home: &Path) {
    // Only signal pids that are genuinely *our* detached supervisor processes
    // (command line names `orchestratectl supervise`). A `supervisor.pid` file
    // can hold an unrelated pid — e.g. `run_error_envelopes` parks the test's
    // own pid to exercise the "supervisor already running" refusal — and a pid
    // can be recycled to a stranger after the supervisor exits; signalling
    // either would be a serious bug (we would SIGTERM the test runner itself).
    let pids: Vec<libc::pid_t> = scan_supervisor_pids(home)
        .into_iter()
        .filter(|&p| is_supervisor_process(p))
        .collect();
    if pids.is_empty() {
        return;
    }
    let our_pgid = unsafe { libc::getpgrp() };
    // Phase 1 — polite signal. A detached supervisor lives in its own session
    // (pgid != ours), so signal the whole process group and take down anything
    // it forked (child supervisors, create.sh helpers). A supervisor that is
    // *not* detached still shares our group, so signal only its pid — never
    // the group, or we would SIGTERM the test runner itself.
    for &pid in &pids {
        if process_gone(pid) {
            continue;
        }
        signal_target(pid, our_pgid, libc::SIGTERM);
    }
    // Phase 2 — escalate to SIGKILL on any that ignored SIGTERM. Identity-safe:
    // the moment `kill(pid, 0)` reports the pid gone or recycled to another
    // owner we stop, so we never SIGKILL a stranger.
    let deadline = Instant::now() + REAP_GRACE;
    for &pid in &pids {
        while !process_gone(pid) {
            if Instant::now() >= deadline {
                signal_target(pid, our_pgid, libc::SIGKILL);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Signal `pid`'s detached process group when it has one distinct from ours,
/// otherwise just the pid. `getpgid` failing (e.g. the pid just exited) falls
/// back to a per-pid signal.
fn signal_target(pid: libc::pid_t, our_pgid: libc::pid_t, sig: libc::c_int) {
    let group = unsafe { libc::getpgid(pid) };
    if group > 1 && group != our_pgid {
        // Detached session created by the supervisor's `setsid`: the group
        // holds only its own lineage, so a group-wide signal is safe.
        unsafe { libc::kill(-group, sig) };
    } else {
        unsafe { libc::kill(pid, sig) };
    }
}

/// True once `pid` no longer names a process we may signal: `kill(pid, 0)`
/// fails with `ESRCH` (gone) or `EPERM` (recycled to another owner). Either
/// way our supervisor is gone and we must not escalate a signal to whatever
/// now holds the pid.
fn process_gone(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) != 0 }
}

/// True iff `pid` names a live `orchestratectl supervise <run-id>` process —
/// the precise command our supervisors run. Matched via `ps` (portable across
/// macOS and Linux), so a parked test pid or a recycled pid (whose command is
/// the test binary or something unrelated) is never mistaken for a supervisor.
/// The `" supervise"` arg distinguishes a real supervisor from the
/// `supervise_gates` *test* binary, whose path also contains "supervise".
fn is_supervisor_process(pid: libc::pid_t) -> bool {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    out.status.success()
        && String::from_utf8_lossy(&out.stdout).contains("orchestratectl supervise")
}

/// Collect the deduplicated, positive supervisor PIDs recorded under
/// `<home>/runs/*/supervisor.pid`.
fn scan_supervisor_pids(home: &Path) -> Vec<libc::pid_t> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir(home.join("runs")) else {
        return pids;
    };
    for entry in entries.flatten() {
        let pid_file = entry.path().join("supervisor.pid");
        if let Some(pid) = read_first_token_pid(&pid_file) {
            if pid > 0 && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

/// Read the first whitespace-delimited token of a pid file as a `pid_t`. The
/// file holds `"<pid> <start_time>"` (or a legacy bare `"<pid>"`).
fn read_first_token_pid(path: &Path) -> Option<libc::pid_t> {
    let s = std::fs::read_to_string(path).ok()?;
    s.split_whitespace().next()?.parse::<libc::pid_t>().ok()
}
