//! Directory layout helpers for a single run.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::schema::{DiscussionId, NodeId, ProposalId, RunId};

/// Reject `path` when it exists and is a symlink — best-effort containment so a
/// replaced run-tree component cannot redirect a read or write outside the run
/// directory. The error is built by `mk_err`, letting callers attach the right
/// variant (run dir / subdir / file). An absent path is accepted: a
/// not-yet-created file or subdir is normal, and the caller's open will fault or
/// create it as usual. Any other `symlink_metadata` failure surfaces as
/// [`Error::Io`].
///
/// **Residual TOCTOU gap.** This is check-then-open: a pure TOCTOU attacker can
/// swap `path` for a symlink in the window between this `symlink_metadata` call
/// and the caller's subsequent open. Closing that gap needs `O_NOFOLLOW` /
/// `openat2` (`RESOLVE_BENEATH` / `RESOLVE_NO_SYMLINKS`), which the standard
/// library does not expose portably. It is out of scope for the MVP threat
/// model: the state root is `$HOME/.orchestratectl/`, a per-user `0700`
/// directory with no shared writers.
pub(crate) fn reject_symlink(path: &Path, mk_err: impl FnOnce() -> Error) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => Err(mk_err()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Validate that `run_id` is a lowercase, ULID-shaped Crockford base32 string.
///
/// Thin wrapper over [`RunId::parse_str`] kept for the `validate_run_id` call
/// sites that only need a yes/no answer in [`crate::Error`] terms. The
/// constraint mirrors what [`crate::new_run_id`] emits: 26 lowercase Crockford
/// base32 characters whose first character keeps the encoded timestamp within
/// ULID's 48-bit range. Storing only validated ids lets the event envelope
/// carry `run_id` directly instead of re-deriving it from a (possibly
/// symlinked or non-canonical) directory name.
pub fn validate_run_id(run_id: &str) -> Result<()> {
    RunId::parse_str(run_id)
        .map(|_| ())
        .map_err(|e| Error::InvalidRunId {
            run_id: run_id.to_string(),
            reason: e.to_string(),
        })
}

/// Per-run paths anchored on `<root>/runs/<run-id>/`.
pub struct RunPaths {
    /// The run's root directory; every other path is derived from it.
    pub root: PathBuf,
    /// The validated run id this directory belongs to. Carried explicitly so
    /// event envelopes never re-derive it from `root.file_name()`.
    pub run_id: RunId,
}

impl RunPaths {
    /// Construct paths for `root` (the run directory) carrying a validated
    /// `run_id`. Rejects malformed ids up front so every downstream event
    /// envelope and projection is stamped with a well-formed id, and rejects a
    /// run root that is a symlink ([`Error::SymlinkRunDir`]) so a replaced run
    /// directory cannot redirect writes outside the run tree. An absent root is
    /// fine — a fresh run is created later; only an existing *symlink* is
    /// refused. See [`reject_symlink`] for the best-effort/TOCTOU caveat.
    pub fn new(root: impl Into<PathBuf>, run_id: impl Into<String>) -> Result<Self> {
        let run_id = run_id.into();
        let rid = RunId::parse_str(&run_id).map_err(|e| Error::InvalidRunId {
            run_id,
            reason: e.to_string(),
        })?;
        let root = root.into();
        reject_symlink(&root, || Error::SymlinkRunDir { path: root.clone() })?;
        Ok(Self { root, run_id: rid })
    }

    /// Construct paths from an already-validated [`RunId`], skipping the
    /// re-parse [`RunPaths::new`] does. `root` must be the run directory
    /// (typically [`run_dir`]'s output for this same id).
    pub fn from_validated(root: impl Into<PathBuf>, run_id: RunId) -> Self {
        Self {
            root: root.into(),
            run_id,
        }
    }

    /// Reject this run's root if it is a symlink ([`Error::SymlinkRunDir`]).
    ///
    /// [`RunPaths::new`] already runs this check, but the production CLI builds
    /// paths via [`RunPaths::from_validated`] (which skips it for speed), so the
    /// projection read/write helpers re-guard the root at access time. Cheap
    /// (one `symlink_metadata`) and best-effort — see [`reject_symlink`].
    pub(crate) fn guard_root(&self) -> Result<()> {
        reject_symlink(&self.root, || Error::SymlinkRunDir {
            path: self.root.clone(),
        })
    }

    /// Path to the run manifest (`manifest.json`).
    pub fn manifest(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Path to the append-only event log (`events.jsonl`).
    pub fn events(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    /// Path to the advisory `flock` file (`.lock`) guarding this run.
    pub fn lock(&self) -> PathBuf {
        self.root.join(".lock")
    }

    /// Path to the `nodes/` directory holding per-node projection files.
    pub fn nodes_dir(&self) -> PathBuf {
        self.root.join("nodes")
    }

    /// Path to a single node's projection file (`nodes/<node-id>.json`).
    ///
    /// Takes a validated [`NodeId`], so the filename can never contain `/` or
    /// `..` and the result can never escape `nodes/`.
    pub fn node(&self, node_id: &NodeId) -> PathBuf {
        self.nodes_dir().join(format!("{}.json", node_id.as_str()))
    }

    /// Path to the `discussions/` directory.
    pub fn discussions_dir(&self) -> PathBuf {
        self.root.join("discussions")
    }

    /// Path to a single discussion file (`discussions/<id>.json`).
    ///
    /// Takes a validated [`DiscussionId`], so the result can never escape
    /// `discussions/`.
    pub fn discussion(&self, id: &DiscussionId) -> PathBuf {
        self.discussions_dir().join(format!("{}.json", id.as_str()))
    }

    /// Path to the `spinoffs/` directory.
    pub fn spinoffs_dir(&self) -> PathBuf {
        self.root.join("spinoffs")
    }

    /// Path to a single spin-off proposal file (`spinoffs/<id>.json`).
    ///
    /// Takes a validated [`ProposalId`], so the result can never escape
    /// `spinoffs/`.
    pub fn spinoff(&self, id: &ProposalId) -> PathBuf {
        self.spinoffs_dir().join(format!("{}.json", id.as_str()))
    }

    /// Path to the supervisor pid file (`supervisor.pid`).
    pub fn supervisor_pid(&self) -> PathBuf {
        self.root.join("supervisor.pid")
    }
}

/// Compose the standard run directory under `<root>/runs/<run-id>`.
///
/// Takes a validated [`RunId`] so this run-level path constructor cannot be
/// handed a `..` or absolute component — closing the same traversal vector the
/// per-run [`RunPaths`] helpers close for node/discussion/spinoff ids.
pub fn run_dir(root: &Path, run_id: &RunId) -> PathBuf {
    root.join("runs").join(run_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_freshly_generated_run_id() {
        let id = crate::new_run_id();
        assert!(
            validate_run_id(&id).is_ok(),
            "generator must satisfy validator: {id}"
        );
        let paths = RunPaths::new("/tmp/x", id.clone()).expect("valid run_id");
        assert_eq!(paths.run_id.as_str(), id);
    }

    #[test]
    fn validator_stays_in_lockstep_with_the_generator() {
        // Guards against drift between `new_run_id()` and the hand-rolled
        // validator: every id the generator can emit must validate, including
        // ones whose timestamp pushes the first character toward the bound.
        for _ in 0..2000 {
            let id = crate::new_run_id();
            assert!(validate_run_id(&id).is_ok(), "generator emitted {id:?}");
        }
    }

    #[test]
    fn accepts_the_first_char_boundary_and_rejects_just_past_it() {
        assert!(validate_run_id("7zzzzzzzzzzzzzzzzzzzzzzzzz").is_ok());
        assert!(matches!(
            RunPaths::new("/tmp/x", "8zzzzzzzzzzzzzzzzzzzzzzzzz"),
            Err(Error::InvalidRunId { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_run_dir_at_construction() {
        // A symlink to a real directory: the id is well-formed, but the run
        // root is a symlink, so `new` must refuse to follow it.
        use std::os::unix::fs::symlink;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            RunPaths::new(&link, "01jxsnap000000000000000000"),
            Err(Error::SymlinkRunDir { path }) if path == link
        ));
    }

    #[test]
    fn accepts_a_real_directory_run_root() {
        // A real (non-symlink) existing directory is fine — only symlinks are
        // refused, not pre-existing run dirs.
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(RunPaths::new(&dir, "01jxsnap000000000000000000").is_ok());
    }

    #[test]
    fn rejects_malformed_run_ids_at_construction() {
        for bad in [
            "tooshort",                    // wrong length
            "01jxsnap0000000000000000000", // 27 chars, too long
            "01JXSNAP000000000000000000",  // uppercase
            "01jxiiiiiiiiiiiiiiiiiiiiii",  // `i` not in Crockford alphabet
            "80000000000000000000000000",  // first char exceeds ULID range
        ] {
            assert!(
                matches!(
                    RunPaths::new("/tmp/x", bad),
                    Err(Error::InvalidRunId { .. })
                ),
                "expected {bad:?} to be rejected",
            );
        }
    }
}
