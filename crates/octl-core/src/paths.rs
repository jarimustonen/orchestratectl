//! Directory layout helpers for a single run.

use std::path::{Path, PathBuf};

/// Per-run paths anchored on `<root>/runs/<run-id>/`.
pub struct RunPaths {
    pub root: PathBuf,
}

impl RunPaths {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: run_dir.into(),
        }
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    pub fn events(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join(".lock")
    }

    pub fn nodes_dir(&self) -> PathBuf {
        self.root.join("nodes")
    }

    pub fn node(&self, node_id: &str) -> PathBuf {
        self.nodes_dir().join(format!("{node_id}.json"))
    }

    pub fn discussions_dir(&self) -> PathBuf {
        self.root.join("discussions")
    }

    pub fn discussion(&self, id: &str) -> PathBuf {
        self.discussions_dir().join(format!("{id}.json"))
    }

    pub fn spinoffs_dir(&self) -> PathBuf {
        self.root.join("spinoffs")
    }

    pub fn spinoff(&self, id: &str) -> PathBuf {
        self.spinoffs_dir().join(format!("{id}.json"))
    }

    pub fn supervisor_pid(&self) -> PathBuf {
        self.root.join("supervisor.pid")
    }
}

/// Compose the standard run directory under `<root>/runs/<run-id>`.
pub fn run_dir(root: &Path, run_id: &str) -> PathBuf {
    root.join("runs").join(run_id)
}
