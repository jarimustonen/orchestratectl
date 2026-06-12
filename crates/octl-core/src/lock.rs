//! Per-run advisory `flock` primitive (design.md §4).

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs2::FileExt;

use crate::error::{Error, Result};

/// RAII guard holding the exclusive `flock` for the run.
///
/// Released on drop. Held across all writes for a single logical mutation.
pub struct RunLock {
    file: Option<File>,
}

impl RunLock {
    /// Acquire the exclusive lock on `<run-dir>/.lock`, creating the file if
    /// needed. Blocks until the lock is available.
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        if let Some(p) = lock_path.parent() {
            std::fs::create_dir_all(p).map_err(|e| Error::io(p, e))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| Error::io(lock_path, e))?;
        file.lock_exclusive().map_err(|e| Error::io(lock_path, e))?;
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
            // Use the fs2 trait method explicitly to avoid clashing with
            // `std::fs::File::unlock` (stable since 1.89, above our MSRV).
            let _ = <File as FileExt>::unlock(&f);
        }
    }
}
