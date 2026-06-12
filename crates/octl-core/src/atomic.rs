//! Atomic write helpers (create-tempfile-then-rename) for projection files.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};

/// Per-process monotonic suffix that disambiguates concurrent in-process
/// writers of the same projection path (the per-run `flock` only serializes
/// across processes; in-process writers must self-disambiguate).
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically: tempfile in the same directory + rename
/// + parent-directory `fsync`. The tempfile is `fsync`ed before rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        Error::IoBare(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path {} has no parent directory", path.display()),
        ))
    })?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let fname = path
        .file_name()
        .ok_or_else(|| {
            Error::IoBare(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("path {} has no file name", path.display()),
            ))
        })?
        .to_string_lossy();
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.tmp.{}.{seq}", std::process::id()));
    {
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| Error::io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| Error::io(&tmp, e))?;
        f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
    // Best-effort parent-directory fsync so the rename survives a power-loss
    // event on filesystems that don't journal directory entries automatically
    // (ext4 without `dirsync`, btrfs without explicit fsync).
    if let Ok(dir_file) = File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

/// JSON-pretty-serialize `value` and atomically write to `path`.
pub fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| Error::json(path, e))?;
    write_atomic(path, &bytes)
}

/// Open `events.jsonl` for `O_APPEND` writes (creating it if absent).
pub fn open_events_append(path: &Path) -> Result<File> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| Error::io(p, e))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| Error::io(path, e))
}
