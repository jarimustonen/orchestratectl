//! Wire DTOs for the `run` noun.
//!
//! These decouple the CLI `--json` contract from the on-disk projection
//! (`octl_core::Manifest`). Handlers serialize a `*View` — never the
//! `Manifest` — so the disk schema can evolve without leaking into the
//! public envelope.
//!
//! Deliberately dropped from the wire contract:
//!
//! - `applied_seq` — the reducer's projection watermark (highest event
//!   `seq` whose fold is durably committed). Pure crash-atomicity
//!   bookkeeping on a derived-cache file; meaningless to a wire consumer.
//!
//! `kind` / `lifecycle` / `status` render through the kebab helpers in
//! [`crate::run`] so a new enum variant is a compile error here, not a
//! silent wire change.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{Manifest, NodeId, RunId};

use super::{kind_kebab, lifecycle_kebab, status_kebab};

/// Full single-run manifest wire view (`run show --json`, nested under
/// `data.manifest`).
///
/// Borrows from the projection: the `show` handler holds the `Manifest`
/// for the lifetime of the emit. Field order and names mirror the
/// established wire contract; the internal `applied_seq` watermark is
/// intentionally absent (see module docs).
#[derive(Serialize)]
pub struct ManifestView<'a> {
    pub schema_version: u32,
    pub run_id: &'a RunId,
    pub kind: &'static str,
    pub lifecycle: &'static str,
    pub title: &'a str,
    pub status: &'static str,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source_repo: Option<&'a str>,
    pub source_branch: Option<&'a str>,
    pub worktree_root: Option<&'a str>,
    pub node_count: u32,
    pub open_discussions: u32,
    pub pending_spinoffs: u32,
    pub parent_run_id: Option<&'a RunId>,
    pub parent_node_id: Option<&'a NodeId>,
}

impl<'a> From<&'a Manifest> for ManifestView<'a> {
    fn from(m: &'a Manifest) -> Self {
        Self {
            schema_version: m.schema_version,
            run_id: &m.run_id,
            kind: kind_kebab(m.kind),
            lifecycle: lifecycle_kebab(m.lifecycle),
            title: &m.title,
            status: status_kebab(m.status),
            created_at: m.created_at,
            updated_at: m.updated_at,
            source_repo: m.source_repo.as_deref(),
            source_branch: m.source_branch.as_deref(),
            worktree_root: m.worktree_root.as_deref(),
            node_count: m.node_count,
            open_discussions: m.open_discussions,
            pending_spinoffs: m.pending_spinoffs,
            parent_run_id: m.parent_run_id.as_ref(),
            parent_node_id: m.parent_node_id.as_ref(),
        }
    }
}

/// One row of `run list --json`.
///
/// Owned: built inside each run's lock from a short-lived manifest.
#[derive(Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub node_count: u32,
}

impl From<&Manifest> for RunSummary {
    fn from(m: &Manifest) -> Self {
        Self {
            run_id: m.run_id.to_string(),
            kind: kind_kebab(m.kind).to_string(),
            status: status_kebab(m.status).to_string(),
            title: m.title.clone(),
            created_at: m.created_at,
            node_count: m.node_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::{Kind, Lifecycle, Status};
    use serde_json::json;

    fn ts() -> DateTime<Utc> {
        "2024-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample() -> Manifest {
        Manifest {
            schema_version: 1,
            applied_seq: 1,
            run_id: RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            kind: Kind::Spinoff,
            lifecycle: Lifecycle::Autonomous,
            title: "seed-run".to_string(),
            status: Status::Pending,
            created_at: ts(),
            updated_at: ts(),
            source_repo: None,
            source_branch: None,
            worktree_root: None,
            managed_tmux_session: None,
            node_count: 0,
            open_discussions: 0,
            pending_spinoffs: 0,
            parent_run_id: None,
            parent_node_id: None,
        }
    }

    #[test]
    fn view_pins_wire_shape() {
        let m = sample();
        let got = serde_json::to_value(ManifestView::from(&m)).unwrap();
        assert_eq!(
            got,
            json!({
                "schema_version": 1,
                "run_id": "01arz3ndektsv4rrffq69g5fav",
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "seed-run",
                "status": "pending",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "source_repo": null,
                "source_branch": null,
                "worktree_root": null,
                "node_count": 0,
                "open_discussions": 0,
                "pending_spinoffs": 0,
                "parent_run_id": null,
                "parent_node_id": null,
            })
        );
    }

    /// `applied_seq` is the reducer watermark, not wire state: bumping it on
    /// the projection leaves the DTO output byte-identical.
    #[test]
    fn applied_seq_does_not_leak() {
        let base = serde_json::to_value(ManifestView::from(&sample())).unwrap();
        let mut bumped = sample();
        bumped.applied_seq = 999;
        let after = serde_json::to_value(ManifestView::from(&bumped)).unwrap();
        assert_eq!(base, after, "applied_seq leaked into run DTO");
        assert!(
            after.get("applied_seq").is_none(),
            "applied_seq must be absent from the wire contract"
        );
    }

    #[test]
    fn summary_pins_wire_shape() {
        let m = sample();
        let got = serde_json::to_value(RunSummary::from(&m)).unwrap();
        assert_eq!(
            got,
            json!({
                "run_id": "01arz3ndektsv4rrffq69g5fav",
                "kind": "spinoff",
                "status": "pending",
                "title": "seed-run",
                "created_at": "2024-01-01T00:00:00Z",
                "node_count": 0,
            })
        );
    }
}
