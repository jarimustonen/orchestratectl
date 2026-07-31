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
//!
//! ## Reservation lifecycle and its limits
//!
//! [`reserve`] is the authoritative check-and-set: it publishes the key
//! atomically (via `hard_link` of a fully-written temp file) BEFORE the
//! run materializes, so a concurrent second call with the same key
//! observes it and replays instead of spawning a duplicate. The caller
//! wraps the reservation in a drop-guard that [`release`]s it on any
//! error before the run is durable, so an early `?` return does not
//! strand the key. Release is ownership-checked (only unlinks a key that
//! still points at this run's id), so a stale release cannot clobber a
//! newer owner's reservation.
//!
//! Known limit (documented, not solved here): a hard process kill
//! (SIGKILL / OOM / power loss) BETWEEN a successful `reserve` and the
//! first durable run artifact leaves a key pointing at a run that was
//! never materialized — the drop-guard cannot run. A later keyed retry
//! then replays that phantom via [`lookup`]. Recovery today is deleting
//! the key file; a full fix needs a PID/timestamp lease or a per-key
//! `flock` two-phase (reserved → committed) protocol, which is a larger
//! redesign than this duplicate-create bug warranted. The `hard_link`
//! primitive also requires a filesystem that supports hard links (every
//! realistic `~/.orchestratectl` target — ext4/btrfs/xfs/apfs/tmpfs —
//! does; FAT/exFAT and some network mounts do not).

use std::io::Write;
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
#[must_use = "a Reservation must be inspected: ignoring AlreadyReserved would duplicate the run"]
pub enum Reservation {
    /// This caller won the key: it now owns `run_id` and must materialize it.
    Reserved,
    /// The key was already reserved by an earlier (possibly still in-flight)
    /// call; the wrapped run-id is the one to replay.
    AlreadyReserved(String),
}

/// Drop-removes a temp file on every exit path from [`reserve_in`], so a read
/// error (or any early return) after the temp is written never leaks it.
struct TempFileGuard(PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// fsync a directory so a newly-created (or removed) entry within it survives a
/// crash. Best-effort: platforms that reject `fsync` on a directory handle
/// (some non-Unix filesystems) simply skip it — the reservation stays correct
/// for the in-process concurrency case, which needs visibility, not durability.
fn fsync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
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
    let parent = root.join("idempotency");
    let p = parent.join(key_hash(repo, branch, key));
    std::fs::create_dir_all(&parent)
        .map_err(|e| CliError::system("io_error", format!("mkdir {}: {}", parent.display(), e)))?;
    // Write the run-id to a temp file, fully and fsynced, before linking it into
    // place. Temp name is disambiguated by run_id (globally unique), so
    // concurrent reservers never collide on the temp file itself; the guard
    // removes it on every exit path.
    let tmp = parent.join(format!(".tmp-{run_id}"));
    let _tmp_guard = TempFileGuard(tmp.clone());
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| {
            CliError::system("io_error", format!("create {}: {}", tmp.display(), e))
        })?;
        f.write_all(run_id.as_bytes())
            .map_err(|e| CliError::system("io_error", format!("write {}: {}", tmp.display(), e)))?;
        // Durability: the reserved key must survive a crash so a post-crash
        // retry replays rather than duplicating. (Visibility to a concurrent
        // in-process caller does not need this — the link is visible at once —
        // but crash-consistency does.)
        f.sync_all()
            .map_err(|e| CliError::system("io_error", format!("fsync {}: {}", tmp.display(), e)))?;
    }
    // Retry loop: the AlreadyExists → read window can observe a concurrent
    // `release` (owner failed and unlinked the key between our failed link and
    // our read). A NotFound there means the key is free again — loop and
    // re-attempt the link rather than surfacing a spurious error. Bounded so a
    // pathological churn cannot spin forever.
    for _ in 0..64 {
        match std::fs::hard_link(&tmp, &p) {
            Ok(()) => {
                // fsync the directory so the new link entry is durable.
                fsync_dir(&parent);
                return Ok(Reservation::Reserved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match std::fs::read_to_string(&p) {
                    Ok(existing) => {
                        return Ok(Reservation::AlreadyReserved(existing.trim().to_string()));
                    }
                    // The owner released the key before we could read it — loop
                    // back and re-attempt the link (the key is free again).
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(CliError::system(
                            "io_error",
                            format!("read {}: {}", p.display(), e),
                        ));
                    }
                }
            }
            Err(e) => {
                return Err(CliError::system(
                    "io_error",
                    format!("link {} -> {}: {}", tmp.display(), p.display(), e),
                ));
            }
        }
    }
    Err(CliError::system(
        "io_error",
        format!(
            "reserve {}: contended past retry budget (repeated create/release churn)",
            p.display()
        ),
    ))
}

/// Release a reservation made by [`reserve`], but ONLY if it still points at
/// `run_id`. Used when a reserved run fails to materialize and its on-disk run
/// dir is discarded (a child-spawn failure, or any error before the run is
/// durable), so a later keyed retry re-spawns cleanly rather than replaying a
/// run that no longer exists.
///
/// The ownership check makes release race-safe: it never clobbers a DIFFERENT
/// run's reservation. Once a key holds some other run's id, only that run can
/// have written it (re-reservation requires the file to be absent first, and a
/// run only ever releases its OWN id), so a stale/late release is a no-op
/// rather than an ABA that would re-open the duplicate-create window. A missing
/// or already-reassigned file is not an error.
pub fn release(
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
    run_id: &str,
) -> Result<(), CliError> {
    let root = home::root_dir()?;
    release_in(&root, repo, branch, key, run_id)
}

fn release_in(
    root: &Path,
    repo: Option<&str>,
    branch: Option<&str>,
    key: &str,
    run_id: &str,
) -> Result<(), CliError> {
    let p = file_path_in(root, repo, branch, key);
    match std::fs::read_to_string(&p) {
        // Only OUR reservation may be removed. A non-match means the key was
        // already released and re-reserved by another run — leave it.
        Ok(s) if s.trim() == run_id => match std::fs::remove_file(&p) {
            Ok(()) => {
                fsync_dir(p.parent().unwrap_or(root));
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CliError::system(
                "io_error",
                format!("remove {}: {}", p.display(), e),
            )),
        },
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("read {}: {}", p.display(), e),
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
        let a = "01runaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "01runbbbbbbbbbbbbbbbbbbbbbbbbb";
        let c = "01runccccccccccccccccccccccccc";

        // Releasing an unreserved key is fine.
        release_in(root, None, None, key, a).unwrap();

        assert!(matches!(
            reserve_in(root, None, None, key, a).unwrap(),
            Reservation::Reserved
        ));
        // Now reserved: a second reserve loses.
        assert!(matches!(
            reserve_in(root, None, None, key, b).unwrap(),
            Reservation::AlreadyReserved(_)
        ));
        // Release by the owner, then a fresh reserve wins again.
        release_in(root, None, None, key, a).unwrap();
        assert!(matches!(
            reserve_in(root, None, None, key, c).unwrap(),
            Reservation::Reserved
        ));
    }

    /// Ownership-checked release: a stale release from a run that no longer
    /// owns the key must NOT clobber the current owner's reservation (else the
    /// duplicate-create window re-opens).
    #[test]
    fn release_is_ownership_checked() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let key = "cas-key";
        let owner = "01runaaaaaaaaaaaaaaaaaaaaaaaaa";
        let stale = "01runbbbbbbbbbbbbbbbbbbbbbbbbb";

        assert!(matches!(
            reserve_in(root, None, None, key, owner).unwrap(),
            Reservation::Reserved
        ));
        // A release naming a DIFFERENT run is a no-op — the owner's key survives.
        release_in(root, None, None, key, stale).unwrap();
        assert_eq!(
            lookup_in(root, None, None, key).unwrap().as_deref(),
            Some(owner)
        );
        // A concurrent reserve still loses (key intact).
        assert!(matches!(
            reserve_in(root, None, None, key, stale).unwrap(),
            Reservation::AlreadyReserved(_)
        ));
    }
}
