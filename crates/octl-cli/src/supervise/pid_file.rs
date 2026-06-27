//! Supervisor PID file: atomic write/read + liveness check.
//!
//! `<run-dir>/supervisor.pid` is the single source of truth for "which
//! supervisor owns this run". A startup check refuses to launch if the
//! file exists AND the recorded PID is still alive. On clean shutdown
//! the supervisor removes the file. After a crash the file is stale and
//! `run reattach` (or a manual `kill`) is required.

use std::path::Path;

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
}
