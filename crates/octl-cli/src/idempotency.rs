//! `--idempotency-key` storage for `run create`.
//!
//! One file per `(source_repo, source_branch, key)` triple under
//! `<root>/idempotency/<hash>`. File contents are the resolved run-id
//! that the first call returned; a second call with the same key
//! returns that run-id and short-circuits.
//!
//! The hash is a 64-bit `DefaultHasher` digest rendered as 16 hex
//! chars. Collisions across distinct `(repo, branch, key)` triples
//! would mis-route a retry to the wrong run-id, but the birthday-bound
//! for that on a single workstation's worth of keys is astronomical;
//! upgrading to a cryptographic digest only matters if we ever share
//! this file pool across machines.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::error::CliError;
use crate::home;

fn key_hash(repo: Option<&str>, branch: Option<&str>, key: &str) -> String {
    let mut h = DefaultHasher::new();
    repo.unwrap_or("").hash(&mut h);
    branch.unwrap_or("").hash(&mut h);
    key.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn file_path(repo: Option<&str>, branch: Option<&str>, key: &str) -> Result<PathBuf, CliError> {
    let root = home::root_dir()?;
    Ok(root.join("idempotency").join(key_hash(repo, branch, key)))
}

pub fn lookup(
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
) -> Result<Option<String>, CliError> {
    let p = file_path(repo, branch, key)?;
    match std::fs::read_to_string(&p) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("read {}: {}", p.display(), e),
        )),
    }
}

pub fn store(
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
    run_id: &str,
) -> Result<(), CliError> {
    let p = file_path(repo, branch, key)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CliError::system("io_error", format!("mkdir {}: {}", parent.display(), e))
        })?;
    }
    std::fs::write(&p, run_id)
        .map_err(|e| CliError::system("io_error", format!("write {}: {}", p.display(), e)))?;
    Ok(())
}
