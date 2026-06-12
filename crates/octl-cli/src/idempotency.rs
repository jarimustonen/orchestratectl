//! `--idempotency-key` storage for `run create`.
//!
//! One file per `(source_repo, source_branch, key)` triple under
//! `<root>/idempotency/<hash>`. File contents are the resolved run-id
//! that the first call returned; a second call with the same key
//! returns that run-id and short-circuits.
//!
//! The hash is a 128-bit FNV-1a digest rendered as 32 hex chars.
//! FNV-1a is chosen over `std::hash::DefaultHasher` because the latter
//! is explicitly documented as not stable across Rust versions — a
//! rustc upgrade would silently invalidate every persisted idempotency
//! key. FNV-1a is a stable, dependency-free algorithm.
//!
//! Collisions across distinct `(repo, branch, key)` triples would
//! mis-route a retry to the wrong run-id, but the 128-bit space makes
//! that essentially impossible on a single workstation. The components
//! are length-prefixed before hashing to avoid the canonical FNV
//! ambiguity (`a||bc` vs. `ab||c`).

use std::path::PathBuf;

use crate::error::CliError;
use crate::home;

const FNV_OFFSET_LOW: u64 = 0xcbf29ce484222325;
const FNV_OFFSET_HIGH: u64 = 0x84222325cbf29ce4;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn key_hash(repo: Option<&str>, branch: Option<&str>, key: &str) -> String {
    // Two independent 64-bit FNV-1a passes with different seeds give
    // 128 bits of output without pulling in `sha2` and friends.
    let mut low = FNV_OFFSET_LOW;
    let mut high = FNV_OFFSET_HIGH;
    for part in [repo.unwrap_or(""), branch.unwrap_or(""), key] {
        let len = (part.len() as u64).to_le_bytes();
        low = fnv1a64(low, &len);
        high = fnv1a64(high, &len);
        low = fnv1a64(low, part.as_bytes());
        high = fnv1a64(high, part.as_bytes());
    }
    format!("{:016x}{:016x}", high, low)
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
