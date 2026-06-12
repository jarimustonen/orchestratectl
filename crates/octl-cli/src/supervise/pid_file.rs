//! Supervisor PID file: atomic write/read + liveness check.
//!
//! `<run-dir>/supervisor.pid` is the single source of truth for "which
//! supervisor owns this run". A startup check refuses to launch if the
//! file exists AND the recorded PID is still alive. On clean shutdown
//! the supervisor removes the file. After a crash the file is stale and
//! `run reattach` (or a manual `kill`) is required.

use std::path::Path;

use crate::error::CliError;

/// Atomically write `pid` to `path` via tempfile + rename. Mirrors
/// `octl_core::atomic::write_atomic` without pulling in the JSON layer.
pub fn write_pid(path: &Path, pid: u32) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| {
        CliError::system(
            "io_error",
            format!("pid path {} has no parent", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        CliError::system("io_error", format!("mkdir {}: {}", parent.display(), e))
    })?;
    let tmp = parent.join(format!(".supervisor.pid.tmp.{}", std::process::id()));
    std::fs::write(&tmp, pid.to_string())
        .map_err(|e| CliError::system("io_error", format!("write {}: {}", tmp.display(), e)))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| CliError::system("io_error", format!("rename {}: {}", path.display(), e)))?;
    Ok(())
}

/// Read the recorded supervisor PID from `path`. Returns `None` if the
/// file is absent or its contents do not parse as an integer.
pub fn read_pid(path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    s.trim().parse::<u32>().ok()
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
