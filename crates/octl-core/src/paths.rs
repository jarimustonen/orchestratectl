//! Directory layout helpers for a single run.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Canonical length of a ULID in Crockford base32.
const RUN_ID_LEN: usize = 26;

/// Crockford base32 alphabet in lowercase (excludes `i`, `l`, `o`, `u`).
const CROCKFORD_LOWER: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Validate that `run_id` is a lowercase, ULID-shaped Crockford base32 string.
///
/// The constraint mirrors what [`crate::new_run_id`] emits: 26 lowercase
/// Crockford base32 characters whose first character keeps the encoded
/// timestamp within ULID's 48-bit range. Storing only validated ids lets
/// the event envelope carry `run_id` directly instead of re-deriving it from
/// a (possibly symlinked or non-canonical) directory name.
pub fn validate_run_id(run_id: &str) -> Result<()> {
    let invalid = |reason: String| Error::InvalidRunId {
        run_id: run_id.to_string(),
        reason,
    };
    if run_id.len() != RUN_ID_LEN {
        return Err(invalid(format!(
            "must be {RUN_ID_LEN} characters (got {})",
            run_id.len()
        )));
    }
    for (pos, b) in run_id.bytes().enumerate() {
        if !CROCKFORD_LOWER.contains(&b) {
            return Err(invalid(format!(
                "character {:?} at position {pos} is not lowercase Crockford base32",
                b as char
            )));
        }
    }
    // The first base32 char carries the top 5 bits of the 128-bit ULID; the
    // 48-bit timestamp cannot overflow only if it is in `0..=7`.
    let first = run_id.as_bytes()[0];
    if !(b'0'..=b'7').contains(&first) {
        return Err(invalid(format!(
            "first character {:?} exceeds ULID range (must be 0-7)",
            first as char
        )));
    }
    Ok(())
}

/// Per-run paths anchored on `<root>/runs/<run-id>/`.
pub struct RunPaths {
    /// The run's root directory; every other path is derived from it.
    pub root: PathBuf,
    /// The validated run id this directory belongs to. Carried explicitly so
    /// event envelopes never re-derive it from `root.file_name()`.
    pub run_id: String,
}

impl RunPaths {
    /// Construct paths for `root` (the run directory) carrying a validated
    /// `run_id`. Rejects malformed ids up front so every downstream event
    /// envelope and projection is stamped with a well-formed id.
    pub fn new(root: impl Into<PathBuf>, run_id: impl Into<String>) -> Result<Self> {
        let run_id = run_id.into();
        validate_run_id(&run_id)?;
        Ok(Self {
            root: root.into(),
            run_id,
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
    pub fn node(&self, node_id: &str) -> PathBuf {
        self.nodes_dir().join(format!("{node_id}.json"))
    }

    /// Path to the `discussions/` directory.
    pub fn discussions_dir(&self) -> PathBuf {
        self.root.join("discussions")
    }

    /// Path to a single discussion file (`discussions/<id>.json`).
    pub fn discussion(&self, id: &str) -> PathBuf {
        self.discussions_dir().join(format!("{id}.json"))
    }

    /// Path to the `spinoffs/` directory.
    pub fn spinoffs_dir(&self) -> PathBuf {
        self.root.join("spinoffs")
    }

    /// Path to a single spin-off proposal file (`spinoffs/<id>.json`).
    pub fn spinoff(&self, id: &str) -> PathBuf {
        self.spinoffs_dir().join(format!("{id}.json"))
    }

    /// Path to the supervisor pid file (`supervisor.pid`).
    pub fn supervisor_pid(&self) -> PathBuf {
        self.root.join("supervisor.pid")
    }
}

/// Compose the standard run directory under `<root>/runs/<run-id>`.
pub fn run_dir(root: &Path, run_id: &str) -> PathBuf {
    root.join("runs").join(run_id)
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
        assert_eq!(paths.run_id, id);
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
