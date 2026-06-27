//! Supervisor resume cursor: `<run-dir>/supervisor.state.json`.
//!
//! Holds (a) `last_seq_own` — the highest `seq` consumed from this run's
//! own `events.jsonl` — and (b) `last_processed_report_seq_by_child` —
//! per-child cursor used by the §7.3 reducer for exactly-once
//! consumption across crashes. This file is supervisor-private state:
//! only the single owning supervisor writes it (`write_json_atomic`,
//! tempfile + rename), so it is NOT taken under the run's `flock` — the
//! event log and projections are the shared, lock-guarded store; this is
//! just the owner's resume cursor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use octl_core::atomic::write_json_atomic;

use crate::error::CliError;

const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorState {
    pub schema_version: u32,
    #[serde(default)]
    pub last_seq_own: u64,
    /// `last_seq_by_child[child_run_id]` is the highest `seq` consumed
    /// from that child's `events.jsonl` (any event kind, used to advance
    /// the tail cursor).
    #[serde(default)]
    pub last_seq_by_child: BTreeMap<String, u64>,
    /// `last_processed_report_seq_by_child[child_run_id]` is the highest
    /// `node.report` seq for which the deterministic-ID reducer has
    /// completed all its parent-side writes.
    #[serde(default)]
    pub last_processed_report_seq_by_child: BTreeMap<String, u64>,
    /// Set of `child_run_id` values for which this supervisor has
    /// already forked a child supervisor process.
    #[serde(default)]
    pub spawned_children: BTreeMap<String, u32>,
}

impl Default for SupervisorState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            last_seq_own: 0,
            last_seq_by_child: BTreeMap::new(),
            last_processed_report_seq_by_child: BTreeMap::new(),
            spawned_children: BTreeMap::new(),
        }
    }
}

pub fn state_path(run_dir: &Path) -> PathBuf {
    run_dir.join("supervisor.state.json")
}

pub fn load(run_dir: &Path) -> Result<SupervisorState, CliError> {
    let p = state_path(run_dir);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| CliError::system("io_error", format!("parse {}: {}", p.display(), e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SupervisorState::default()),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("read {}: {}", p.display(), e),
        )),
    }
}

pub fn save(run_dir: &Path, state: &SupervisorState) -> Result<(), CliError> {
    write_json_atomic(&state_path(run_dir), state)
        .map_err(|e| CliError::system("io_error", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_state_returns_default() {
        let dir = TempDir::new().unwrap();
        let s = load(dir.path()).unwrap();
        assert_eq!(s.last_seq_own, 0);
        assert!(s.last_processed_report_seq_by_child.is_empty());
    }

    #[test]
    fn round_trip_preserves_cursors() {
        let dir = TempDir::new().unwrap();
        let mut s = SupervisorState {
            last_seq_own: 42,
            ..Default::default()
        };
        s.last_processed_report_seq_by_child
            .insert("child-1".to_string(), 7);
        s.spawned_children.insert("child-1".to_string(), 999);
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.last_seq_own, 42);
        assert_eq!(
            loaded.last_processed_report_seq_by_child.get("child-1"),
            Some(&7)
        );
        assert_eq!(loaded.spawned_children.get("child-1"), Some(&999));
    }
}
