//! Per-run advisory `flock` primitive (design.md §4).

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;

use fs4::FileExt;

use crate::error::{Error, Result};
use crate::paths::reject_symlink;

/// RAII guard holding the run's `flock`.
///
/// Released on drop. Acquired exclusively ([`RunLock::acquire`]) and held across
/// all writes for a single logical mutation, or shared ([`RunLock::acquire_shared`])
/// for the duration of a reader's multi-file scan so a concurrent reducer cannot
/// leave the reader with a half-updated projection set. A shared guard may hold
/// no lock at all when the run has no `.lock` file yet — see
/// [`RunLock::acquire_shared`].
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

    /// Acquire a **shared** (`LOCK_SH`) lock on an existing `<run-dir>/.lock`,
    /// blocking until no writer holds the exclusive lock. The mirror of
    /// [`RunLock::acquire`] for the read side: many readers may hold the shared
    /// lock at once, but the exclusive lock a reducer takes excludes them all,
    /// so a multi-file read taken under this lock never observes the torn state
    /// a mid-flight reducer would otherwise expose (design.md §4).
    ///
    /// Unlike [`RunLock::acquire`], this **never creates** the run directory or
    /// the lock file — a reader must not bring run-tree state into existence. A
    /// missing `.lock` means no writer has ever locked this run, so a lock-free
    /// read is already coherent: the returned guard then holds nothing and drops
    /// to a no-op. The same symlink containment as [`RunLock::acquire`] applies;
    /// the file is opened read-only with `O_NOFOLLOW`.
    ///
    /// Nesting is safe: a shared lock is compatible with other shared locks, so
    /// a read path that calls another read helper (each on its own descriptor)
    /// cannot deadlock against itself.
    pub fn acquire_shared(lock_path: &Path) -> Result<Self> {
        reject_symlink(lock_path, || Error::SymlinkStateFile {
            name: "lock",
            path: lock_path.to_path_buf(),
        })?;
        let mut opts = OpenOptions::new();
        // Read-only, no `create`: a reader never authors the lock file or its
        // parent run directory.
        opts.read(true);
        // `O_NOFOLLOW`: refuse to take `flock` through a symlinked `.lock`, the
        // file-level backstop to the `reject_symlink` check above.
        crate::paths::nofollow(&mut opts);
        let file = match opts.open(lock_path) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // No `.lock` (or no run dir): nothing to serialize against.
                return Ok(Self { file: None });
            }
            Err(e) => return Err(Error::io(lock_path, e)),
        };
        // Fully-qualified to call fs4's trait method rather than the inherent
        // `std::fs::File::lock_shared` (stable 1.89) that would shadow it on
        // newer toolchains — same reasoning as the exclusive `lock` above.
        <File as FileExt>::lock_shared(&file).map_err(|e| Error::io(lock_path, e))?;
        Ok(Self { file: Some(file) })
    }

    /// Convenience: run `f` with the shared lock held, releasing afterwards.
    /// Mirrors [`RunLock::with_lock`] so a read path can swap one for the other.
    pub fn with_shared_lock<T>(lock_path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let guard = Self::acquire_shared(lock_path)?;
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

    /// A shared lock on a run with no `.lock` file is a no-op guard, not an
    /// error: a reader must not create run-tree state, and a missing lock file
    /// means no writer can race the read anyway.
    #[test]
    fn acquire_shared_on_missing_lock_file_is_a_noop_guard() {
        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join(".lock");
        let guard = RunLock::acquire_shared(&lock).expect("missing lock file is fine");
        assert!(guard.file.is_none(), "no lock file ⇒ guard holds nothing");
        // The read must not have created the lock file (or its parent run dir).
        assert!(!lock.exists(), "a reader must never author the lock file");
    }

    /// A writer holding the exclusive lock blocks a shared reader until release.
    #[test]
    fn exclusive_writer_blocks_shared_reader_until_release() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join(".lock");
        // The writer's exclusive acquire creates the lock file the reader opens.
        let writer = RunLock::acquire(&lock).unwrap();

        let (tx, rx) = mpsc::channel();
        let lock2 = lock.clone();
        let reader = thread::spawn(move || {
            // Blocks until the exclusive lock is released.
            let _g = RunLock::acquire_shared(&lock2).unwrap();
            tx.send(()).unwrap();
        });

        // While the exclusive lock is held, the reader cannot proceed.
        assert!(
            rx.recv_timeout(Duration::from_millis(250)).is_err(),
            "shared reader must block while the exclusive lock is held"
        );
        drop(writer);
        // Once released, the reader acquires the shared lock and reports.
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "shared reader must proceed after the exclusive lock is released"
        );
        reader.join().unwrap();
    }

    /// Two shared readers hold the lock concurrently — neither blocks the other.
    #[test]
    fn two_shared_readers_proceed_concurrently() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join(".lock");
        // Create the lock file (writer makes it, then releases).
        drop(RunLock::acquire(&lock).unwrap());

        // First reader takes and holds the shared lock.
        let r1 = RunLock::acquire_shared(&lock).unwrap();
        assert!(
            r1.file.is_some(),
            "lock file exists ⇒ real shared lock held"
        );

        // A second reader must acquire it without blocking on the first.
        let (tx, rx) = mpsc::channel();
        let lock2 = lock.clone();
        let r2 = thread::spawn(move || {
            let _g = RunLock::acquire_shared(&lock2).unwrap();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "a second shared reader must not block on the first"
        );
        r2.join().unwrap();
    }

    /// Nested `with_shared_lock` calls (each on its own descriptor) must not
    /// deadlock — shared locks are compatible with one another.
    #[test]
    fn nested_shared_locks_do_not_deadlock() {
        let tmp = TempDir::new().unwrap();
        let lock = tmp.path().join(".lock");
        drop(RunLock::acquire(&lock).unwrap());
        let r = RunLock::with_shared_lock(&lock, || RunLock::with_shared_lock(&lock, || Ok(42)))
            .unwrap();
        assert_eq!(r, 42);
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
