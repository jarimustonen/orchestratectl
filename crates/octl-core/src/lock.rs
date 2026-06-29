//! Per-run advisory `flock` primitive (design.md §4).

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs4::FileExt;

use crate::error::{Error, Result};
use crate::paths::reject_symlink;

/// RAII guard holding the exclusive `flock` for the run.
///
/// Released on drop. Held across all writes for a single logical mutation.
pub struct RunLock {
    file: Option<File>,
}

impl RunLock {
    /// Acquire the exclusive lock on `<run-dir>/.lock`, creating the file if
    /// needed. Blocks until the lock is available.
    ///
    /// Best-effort symlink containment: a `.lock` that is a symlink is refused
    /// ([`Error::SymlinkStateFile`]) so `flock` cannot be taken on a file
    /// outside the run tree, which would silently break mutual exclusion. This
    /// guards the lock file's own final component; a symlinked *run root* is
    /// caught downstream when the held critical section opens `events.jsonl` /
    /// the projections (both re-guard the root before writing). See
    /// [`reject_symlink`](crate::paths) for the check-then-open TOCTOU caveat.
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        // Test-only spy: count this acquisition so a test can assert a
        // multi-write transaction (e.g. `cancel_run`) takes the lock exactly
        // once, not once per appended event.
        #[cfg(test)]
        ACQUIRE_COUNT.with(|c| c.set(c.get() + 1));
        if let Some(p) = lock_path.parent() {
            std::fs::create_dir_all(p).map_err(|e| Error::io(p, e))?;
        }
        reject_symlink(lock_path, || Error::SymlinkStateFile {
            name: "lock",
            path: lock_path.to_path_buf(),
        })?;
        let mut opts = OpenOptions::new();
        opts.create(true).read(true).write(true).truncate(false);
        // `O_NOFOLLOW`: refuse to take `flock` through a symlinked `.lock`, the
        // file-level backstop to the `reject_symlink` check above.
        crate::paths::nofollow(&mut opts);
        let file = opts.open(lock_path).map_err(|e| Error::io(lock_path, e))?;
        // Fully-qualified to call fs4's trait method, not `std::fs::File::lock`
        // (an inherent method stable since 1.89 that would otherwise shadow it
        // on newer toolchains). fs4 renamed `fs2`'s `lock_exclusive` to `lock`
        // to mirror std.
        <File as FileExt>::lock(&file).map_err(|e| Error::io(lock_path, e))?;
        Ok(Self { file: Some(file) })
    }

    /// Convenience: run `f` with the lock held, releasing afterwards.
    pub fn with_lock<T>(lock_path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let guard = Self::acquire(lock_path)?;
        let r = f();
        drop(guard);
        r
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        if let Some(f) = self.file.take() {
            // Best-effort unlock — kernel releases on file close anyway.
            // Use the fs4 trait method explicitly to avoid clashing with
            // `std::fs::File::unlock` (stable since 1.89, above our MSRV).
            let _ = <File as FileExt>::unlock(&f);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only spy counter for [`RunLock::acquire`] calls on the current
    /// thread. `cargo test` runs each test on its own thread and `cancel_run`
    /// does all its work synchronously on the calling thread, so a test can
    /// reset this and assert the exact number of lock acquisitions a
    /// transaction performed without cross-test interference.
    pub(crate) static ACQUIRE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_succeeds_on_a_regular_lock_file() {
        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join(".lock");
        // First acquire creates the file; a second acquire after drop succeeds.
        drop(RunLock::acquire(&lock).unwrap());
        assert!(RunLock::acquire(&lock).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn acquire_rejects_a_symlinked_lock_file() {
        // A symlinked `.lock` would take `flock` on a file outside the run,
        // silently breaking mutual exclusion — refuse it.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("outside.lock");
        let lock = tmp.path().join(".lock");
        symlink(&target, &lock).unwrap();
        assert!(matches!(
            RunLock::acquire(&lock),
            Err(Error::SymlinkStateFile { name: "lock", .. })
        ));
        // The forged lock never touched the symlink target.
        assert!(!target.exists());
    }
}
