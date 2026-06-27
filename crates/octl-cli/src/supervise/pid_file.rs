//! Supervisor PID file: atomic write/read + liveness check.
//!
//! `<run-dir>/supervisor.pid` is the single source of truth for "which
//! supervisor owns this run". A startup check refuses to launch if the
//! file exists AND the recorded PID is still alive. On clean shutdown
//! the supervisor removes the file. After a crash the file is stale and
//! `run reattach` (or a manual `kill`) is required.

use std::path::Path;

use octl_core::{RunLock, RunPaths};

use crate::error::CliError;

/// Atomically write `pid` (plus its process start-time, when readable)
/// to `path` via tempfile + rename. The on-disk format is one line:
/// `"<pid>"` or `"<pid> <start_time_secs>"`. The start-time is the §7.5
/// PID-identity defense (§7.6): a later stale-check can tell a recycled
/// PID from the original supervisor. Mirrors `octl_core::atomic` without
/// pulling in the JSON layer.
pub fn write_pid(path: &Path, pid: u32) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| {
        CliError::system(
            "io_error",
            format!("pid path {} has no parent", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|e| CliError::system("io_error", format!("mkdir {}: {}", parent.display(), e)))?;
    let contents = if let Some(st) = crate::supervise::watchdog::pid_start_time(pid) {
        format!("{pid} {st}")
    } else {
        // Degrade to the legacy bare-integer format (a later
        // stale-check falls back to plain liveness). This loses the
        // §7.6 PID-reuse defense, so make it visible rather than
        // silent — we can normally always read our own start-time.
        tracing::warn!(
            target: "orchestratectl::supervise",
            pid,
            "could not read own start_time; writing legacy pid file (no recycle defense)"
        );
        pid.to_string()
    };
    let tmp = parent.join(format!(".supervisor.pid.tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents)
        .map_err(|e| CliError::system("io_error", format!("write {}: {}", tmp.display(), e)))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| CliError::system("io_error", format!("rename {}: {}", path.display(), e)))?;
    Ok(())
}

/// Atomically claim supervisor ownership of `paths`' run under the run
/// `flock`, closing the §7.6 TOCTOU race.
///
/// The original startup check read a stale `supervisor.pid`, then wrote its
/// own with *no lock between* — so two concurrent `supervise` / `run
/// reattach`-spawned supervisors could both read "stale", both write, and
/// both enter the main loop, violating one-supervisor-per-run. Holding the
/// run flock across the read-then-write serializes the claim: exactly one
/// caller observes the slot free and writes; the loser sees the winner's
/// live pid and is rejected.
///
/// Sequence (all under the lock):
///  1. Read the existing `supervisor.pid` record (if any).
///  2. If it records a supervisor still alive (start-time identity check,
///     §7.6) → return `supervisor_already_running` *without* touching the
///     file.
///  3. Otherwise (no file, dead pid, recycled pid, or a legacy plain-integer
///     file) → write our pid + start-time atomically (tempfile + rename) and
///     return `Ok`. A legacy bare-integer file is upgraded to the
///     `"<pid> <start_time>"` format here on first claim — a non-destructive
///     migration that restores the recycle defense for the new owner.
///
/// The lock is released when the returned guard drops at end of scope; the
/// pid file itself remains as the durable ownership marker.
pub fn claim_pid_atomic(paths: &RunPaths, our_pid: u32) -> Result<(), CliError> {
    let _guard = RunLock::acquire(&paths.lock())
        .map_err(|e| CliError::system("lock_error", format!("acquire run lock: {e}")))?;
    let pid_path = paths.supervisor_pid();
    if let Some((existing, start_time)) = read_pid_record(&pid_path) {
        if pid_live_with_identity(existing, start_time) {
            return Err(CliError::system(
                "supervisor_already_running",
                format!(
                    "supervisor pid {existing} for run {} is alive (kill it or use `run reattach`)",
                    paths.run_id.as_str(),
                ),
            ));
        }
        // Stale (dead, or a recycled PID per the start-time check) or a legacy
        // bare-integer file: log and overwrite under the lock.
        tracing::warn!(
            target: "orchestratectl::supervise",
            stale_pid = existing,
            "claiming run: removing stale supervisor.pid"
        );
    }
    write_pid(&pid_path, our_pid)
}

/// Read the recorded supervisor PID from `path`. Returns `None` if the
/// file is absent or its first token does not parse as an integer.
pub fn read_pid(path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    s.split_whitespace().next()?.parse::<u32>().ok()
}

/// Read `(pid, start_time)` from `path`. `start_time` is `None` for a
/// legacy single-integer file (written before §7.6 identity landed) or
/// when the start-time could not be captured at write time.
pub fn read_pid_record(path: &Path) -> Option<(u32, Option<u64>)> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut it = s.split_whitespace();
    let pid = it.next()?.parse::<u32>().ok()?;
    let start_time = it.next().and_then(|t| t.parse::<u64>().ok());
    Some((pid, start_time))
}

/// Liveness check for a recorded supervisor PID that additionally
/// defends against PID reuse via the §7.5 start-time identity check
/// (§7.6). A recycled PID (alive, but start-time disagrees) is reported
/// `false` (stale) so reattach is not blocked forever. A legacy record
/// with no recorded start-time, or a platform that cannot read it, falls
/// back to plain liveness.
pub fn pid_live_with_identity(pid: u32, recorded_start_time: Option<u64>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    match recorded_start_time {
        Some(recorded) => match crate::supervise::watchdog::pid_start_time(pid) {
            // 1s tolerance mirrors the watchdog's recycle check.
            Some(actual) => recorded.abs_diff(actual) <= 1,
            None => true,
        },
        None => true,
    }
}

/// Remove the PID file if it still records `expected_pid`. Mismatch is
/// silent: another supervisor may have taken over.
pub fn remove_if_owner(path: &Path, expected_pid: u32) {
    if read_pid(path) == Some(expected_pid) {
        let _ = std::fs::remove_file(path);
    }
}

/// `kill(pid, 0)` — returns true iff the process exists and we have
/// permission to signal it. `ESRCH` → dead, `EPERM` → alive but
/// foreign-owned (still counts as alive for liveness purposes).
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill(pid, 0)` is signal-free and side-effect-free on
    // POSIX; it only probes existence/permission.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    // Errno EPERM (1 on macOS/Linux) → process exists, foreign owner.
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    errno == libc::EPERM
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_write_read() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("supervisor.pid");
        write_pid(&p, 12345).unwrap();
        assert_eq!(read_pid(&p), Some(12345));
    }

    #[test]
    fn missing_file_reads_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(read_pid(&dir.path().join("missing")), None);
    }

    #[test]
    fn remove_if_owner_matches() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("supervisor.pid");
        write_pid(&p, 999).unwrap();
        remove_if_owner(&p, 1234); // wrong pid: file stays
        assert!(p.exists());
        remove_if_owner(&p, 999);
        assert!(!p.exists());
    }

    #[test]
    fn own_pid_is_alive() {
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn pid_zero_is_dead() {
        assert!(!pid_alive(0));
    }

    /// The core invariant of the §7.6 fix: many concurrent `claim_pid_atomic`
    /// calls racing on one run yield EXACTLY ONE winner; every loser gets
    /// `supervisor_already_running`. All threads claim our own (alive,
    /// identity-matching) pid, so the loser always observes the winner's pid
    /// as live and is rejected.
    #[test]
    fn concurrent_claim_exactly_one_wins() {
        use std::sync::{Arc, Barrier};

        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("01jxsnap000000000000000000");
        std::fs::create_dir_all(&run_dir).unwrap();
        let run_id = "01jxsnap000000000000000000";
        let our_pid = std::process::id();

        const N: usize = 8;
        let barrier = Arc::new(Barrier::new(N));
        let handles: Vec<_> = (0..N)
            .map(|_| {
                let rd = run_dir.clone();
                let b = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let paths = RunPaths::new(rd, run_id).unwrap();
                    // Line every thread up so they genuinely contend on the flock.
                    b.wait();
                    claim_pid_atomic(&paths, our_pid)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "exactly one concurrent claim must win, got {wins}");
        for r in &results {
            if let Err(e) = r {
                assert_eq!(
                    e.code, "supervisor_already_running",
                    "loser must report supervisor_already_running, got {}",
                    e.code
                );
            }
        }

        // The winner's pid is on disk.
        let p = run_dir.join("supervisor.pid");
        assert_eq!(read_pid(&p), Some(our_pid));
    }

    /// A legacy bare-integer pid file (written before the §7.6 start-time
    /// identity format) is non-destructively upgraded to `"<pid> <start>"`
    /// on the next claim, provided the recorded pid is not alive.
    #[test]
    fn claim_migrates_legacy_plain_integer_pid_file() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("01jxsnap000000000000000000");
        std::fs::create_dir_all(&run_dir).unwrap();
        let paths = RunPaths::new(run_dir.clone(), "01jxsnap000000000000000000").unwrap();
        let pid_path = run_dir.join("supervisor.pid");

        // Legacy format: a single bare integer, no start-time token, for a
        // guaranteed-dead pid (so the claim is not blocked by a live owner).
        std::fs::write(&pid_path, "2147483646").unwrap();
        assert_eq!(read_pid_record(&pid_path), Some((2_147_483_646, None)));

        let our_pid = std::process::id();
        claim_pid_atomic(&paths, our_pid).expect("claim over a dead legacy pid succeeds");

        // Rewritten in the modern format: pid + start-time (when readable).
        let (pid, start) = read_pid_record(&pid_path).unwrap();
        assert_eq!(pid, our_pid);
        assert!(
            start.is_some(),
            "claim must upgrade the legacy file to carry a start-time"
        );
    }
}
