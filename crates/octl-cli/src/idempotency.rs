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

use std::path::{Path, PathBuf};

use crate::error::CliError;
use crate::home;

const FNV_OFFSET_LOW: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_OFFSET_HIGH: u64 = 0x8422_2325_cbf2_9ce4;
const FNV_PRIME: u64 = 0x100_0000_01b3;

fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for &b in bytes {
        h ^= u64::from(b);
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
    format!("{high:016x}{low:016x}")
}

/// Path to a key's reservation file under `<root>/idempotency/<hash>`.
///
/// The `_in` functions take the orchestratectl root explicitly so tests can
/// point at a scratch dir without mutating the process-global
/// `ORCHESTRATECTL_HOME` (which would race with other unit tests). The public
/// wrappers (`lookup` / `reserve` / `release`) resolve the root via `home` and
/// delegate.
fn file_path_in(root: &Path, repo: Option<&str>, branch: Option<&str>, key: &str) -> PathBuf {
    root.join("idempotency").join(key_hash(repo, branch, key))
}

pub fn lookup(
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
) -> Result<Option<String>, CliError> {
    let root = home::root_dir()?;
    lookup_in(&root, repo, branch, key)
}

fn lookup_in(
    root: &Path,
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
) -> Result<Option<String>, CliError> {
    let p = file_path_in(root, repo, branch, key);
    match std::fs::read_to_string(&p) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("read {}: {}", p.display(), e),
        )),
    }
}

/// Outcome of an atomic [`reserve`].
pub enum Reservation {
    /// This caller won the key: it now owns `run_id` and must materialize it.
    Reserved,
    /// The key was already reserved by an earlier (possibly still in-flight)
    /// call; the wrapped run-id is the one to replay.
    AlreadyReserved(String),
}

/// Atomically reserve `key` for `run_id`, closing the two-near-simultaneous
/// duplicate-create race.
///
/// The reservation is an atomic filesystem check-and-set: `run_id` is written
/// to a uniquely-named temp file in the idempotency dir and then `hard_link`ed
/// into the key's canonical path. `hard_link` fails with `AlreadyExists` when
/// the key is already reserved, so exactly one of N concurrent callers gets
/// [`Reservation::Reserved`] and the rest observe the winner's reservation.
///
/// Linking a *fully-written* temp file (rather than create-then-write) means a
/// concurrent loser never reads a half-written / empty reservation file: the
/// canonical path only ever appears as a link to complete content.
///
/// Callers must reserve BEFORE materializing the run so a concurrent second
/// call with the same key observes the reservation and replays instead of
/// spawning a duplicate. This mirrors the durable-marker-before-side-effect
/// discipline the supervisor uses for `run.notified`.
pub fn reserve(
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
    run_id: &str,
) -> Result<Reservation, CliError> {
    let root = home::root_dir()?;
    reserve_in(&root, repo, branch, key, run_id)
}

fn reserve_in(
    root: &Path,
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
    run_id: &str,
) -> Result<Reservation, CliError> {
    let p = file_path_in(root, repo, branch, key);
    let parent = p
        .parent()
        .expect("idempotency path always has a parent dir");
    std::fs::create_dir_all(parent)
        .map_err(|e| CliError::system("io_error", format!("mkdir {}: {}", parent.display(), e)))?;
    // Temp name is disambiguated by run_id, which is globally unique, so
    // concurrent reservers never collide on the temp file itself.
    let tmp = parent.join(format!(".tmp-{run_id}"));
    std::fs::write(&tmp, run_id)
        .map_err(|e| CliError::system("io_error", format!("write {}: {}", tmp.display(), e)))?;
    let result = match std::fs::hard_link(&tmp, &p) {
        Ok(()) => Reservation::Reserved,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(&p).map_err(|e| {
                CliError::system("io_error", format!("read {}: {}", p.display(), e))
            })?;
            Reservation::AlreadyReserved(existing.trim().to_string())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(CliError::system(
                "io_error",
                format!("link {} -> {}: {}", tmp.display(), p.display(), e),
            ));
        }
    };
    // The link (or the loser's read) is done; the temp file is no longer
    // needed. Best-effort: a leftover temp file is harmless (unique name).
    let _ = std::fs::remove_file(&tmp);
    Ok(result)
}

/// Release a reservation made by [`reserve`]. Used only when a reserved run
/// fails to materialize and its on-disk run dir is discarded (a child-spawn
/// failure), so a later keyed retry re-spawns cleanly rather than replaying a
/// run that no longer exists. A missing file is not an error (already gone).
pub fn release(repo: Option<&str>, branch: Option<&str>, key: &str) -> Result<(), CliError> {
    let root = home::root_dir()?;
    release_in(&root, repo, branch, key)
}

fn release_in(
    root: &Path,
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
) -> Result<(), CliError> {
    let p = file_path_in(root, repo, branch, key);
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("remove {}: {}", p.display(), e),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden test pinning the `(repo, branch, key) → hash` derivation.
    ///
    /// `key_hash` is the integrity boundary for `--idempotency-key`: a
    /// silent change to the FNV constants, the length-prefix shape, or
    /// the empty-component handling would invalidate every persisted
    /// key on disk and either double-execute a retried call or silently
    /// dedup against the wrong run. Any future drift from these literals
    /// is a wire-incompatible change and needs a migration plan, not a
    /// test update (issue: idempotency-hash-golden-test).
    #[test]
    fn key_hash_is_stable() {
        assert_eq!(
            key_hash(Some("repo"), Some("main"), "key-1"),
            "374d54a7713c5c1529a2efe850ddaf06"
        );
        assert_eq!(
            key_hash(None, None, "key-1"),
            "63e136949fd4014a7e5f1d5d18d98cc5"
        );
        assert_eq!(
            key_hash(Some("repo"), Some("main"), ""),
            "3b1adbcd4680d8f508f812409f6e4e60"
        );
        assert_eq!(
            key_hash(Some("räpo"), Some("main"), "key-1"),
            "f3b96680018fdf90bf5f9357e0ff987d"
        );
        // Length-prefix anti-canonical-FNV-ambiguity guard: differing
        // splits of the same byte sequence must hash differently.
        assert_ne!(
            key_hash(Some("a"), Some("bc"), "key"),
            key_hash(Some("ab"), Some("c"), "key")
        );
    }

    /// Concurrency contract for `reserve`: N threads racing the same key must
    /// yield exactly ONE `Reserved`; every loser observes the winner's run-id.
    /// This is the invariant that stops two near-simultaneous `run create`
    /// calls from both materializing (issue
    /// `idempotency-key-allowed-duplicate-run`).
    #[test]
    fn reserve_is_atomic_under_concurrency() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let key = "race-key";
        const N: usize = 16;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(N));
        let winners = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..N)
            .map(|i| {
                let barrier = barrier.clone();
                let winners = winners.clone();
                let observed = observed.clone();
                let root = root.clone();
                std::thread::spawn(move || {
                    // Each thread proposes a distinct run-id so we can tell the
                    // single winner's id apart from the losers' proposals.
                    let my_run_id = format!("01run{i:026}");
                    barrier.wait();
                    match reserve_in(&root, None, None, key, &my_run_id).unwrap() {
                        Reservation::Reserved => {
                            winners.lock().unwrap().push(my_run_id.clone());
                            observed.lock().unwrap().push(my_run_id);
                        }
                        Reservation::AlreadyReserved(existing) => {
                            observed.lock().unwrap().push(existing);
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let winners = winners.lock().unwrap();
        assert_eq!(winners.len(), 1, "exactly one thread may win the key");
        let winning_id = &winners[0];
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), N);
        assert!(
            observed.iter().all(|id| id == winning_id),
            "every caller must resolve to the single winner's run-id; got {observed:?}"
        );

        // The persisted key holds the winner's run-id, and a later lookup sees
        // it — the fast path a fully-registered retry uses.
        assert_eq!(
            lookup_in(&root, None, None, key).unwrap().as_deref(),
            Some(winning_id.as_str())
        );
    }

    /// `release` removes a reservation so a subsequent `reserve` wins afresh;
    /// releasing a missing key is a no-op.
    #[test]
    fn release_frees_the_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let key = "release-key";

        // Releasing an unreserved key is fine.
        release_in(root, None, None, key).unwrap();

        assert!(matches!(
            reserve_in(root, None, None, key, "01runaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            Reservation::Reserved
        ));
        // Now reserved: a second reserve loses.
        assert!(matches!(
            reserve_in(root, None, None, key, "01runbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
            Reservation::AlreadyReserved(_)
        ));
        // Release, then a fresh reserve wins again.
        release_in(root, None, None, key).unwrap();
        assert!(matches!(
            reserve_in(root, None, None, key, "01runccccccccccccccccccccccccc").unwrap(),
            Reservation::Reserved
        ));
    }
}
