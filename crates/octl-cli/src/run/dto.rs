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

use octl_core::{Manifest, NodeId, RunId, RunPaths};

use super::{kind_kebab, lifecycle_kebab, status_kebab};
use crate::supervise::pid_file;

/// Liveness of the run's per-run supervisor, surfaced on `run show` /
/// `run list` so a caller can tell "still working" from "orphaned".
///
/// A run whose `status` is still `pending` while `alive` is `false` and a
/// `pid` is recorded is the orphaned condition the
/// `supervisor-dead-merge-no-teardown` bug describes: a supervisor was
/// started, then died (e.g. SIGTERM), and nothing is left to consume the
/// terminal report or run teardown. Recover with `run reattach <id>`.
#[derive(Serialize)]
pub struct SupervisorView {
    /// The supervisor PID recorded in `<run-dir>/supervisor.pid`, or `null`
    /// when no supervisor is recorded (never materialized, or cleanly torn
    /// down — the supervisor removes its pid file on a clean exit).
    pub pid: Option<u32>,
    /// Whether that PID is a live process whose start-time still matches the
    /// record (§7.6 identity check — a recycled PID reads as dead). Always
    /// `false` when `pid` is `null`.
    pub alive: bool,
}

impl SupervisorView {
    /// Probe `<run-dir>/supervisor.pid` for the recorded supervisor and its
    /// liveness. An absent/unreadable file → `{pid: null, alive: false}`.
    ///
    /// Single-file read (the pid file is CLI-owned and does not route through
    /// the run-state projection guards), so it needs no shared lock: it never
    /// participates in a multi-projection decision.
    pub fn probe(paths: &RunPaths) -> Self {
        match pid_file::read_pid_record(&paths.supervisor_pid()) {
            Some((pid, start_time)) => Self {
                pid: Some(pid),
                alive: pid_file::pid_live_with_identity(pid, start_time),
            },
            None => Self {
                pid: None,
                alive: false,
            },
        }
    }

    /// The "no supervisor probed" default the DTO `From` impls carry until a
    /// handler overrides it via `with_supervisor`.
    fn unknown() -> Self {
        Self {
            pid: None,
            alive: false,
        }
    }
}

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
    /// Liveness of the run's per-run supervisor. Defaults to the "unknown"
    /// probe (`{pid: null, alive: false}`) from `From`; the `show` handler
    /// overrides it with a real probe via [`ManifestView::with_supervisor`].
    pub supervisor: SupervisorView,
}

impl ManifestView<'_> {
    /// Attach a probed [`SupervisorView`], replacing the `From`-provided
    /// "unknown" default. Builder so the handler stays a one-liner.
    #[must_use]
    pub fn with_supervisor(mut self, supervisor: SupervisorView) -> Self {
        self.supervisor = supervisor;
        self
    }
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
            supervisor: SupervisorView::unknown(),
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
    /// Liveness of the run's per-run supervisor. Defaults to the "unknown"
    /// probe from `From`; the `list` handler overrides it with a real probe
    /// via [`RunSummary::with_supervisor`] (it already holds each run's paths).
    pub supervisor: SupervisorView,
}

impl RunSummary {
    /// Attach a probed [`SupervisorView`], replacing the `From`-provided
    /// "unknown" default.
    #[must_use]
    pub fn with_supervisor(mut self, supervisor: SupervisorView) -> Self {
        self.supervisor = supervisor;
        self
    }
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
            supervisor: SupervisorView::unknown(),
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
                "supervisor": { "pid": null, "alive": false },
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
                "supervisor": { "pid": null, "alive": false },
            })
        );
    }

    /// `SupervisorView::probe` reads the real `supervisor.pid` file: a live,
    /// identity-matching record reads `alive`, an absent file reads
    /// `{pid: null, alive: false}`.
    #[test]
    fn supervisor_probe_reads_pid_file() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("01arz3ndektsv4rrffq69g5fav");
        std::fs::create_dir_all(&run_dir).unwrap();
        let paths = RunPaths::new(run_dir, "01arz3ndektsv4rrffq69g5fav").unwrap();

        // No pid file yet → unknown.
        let v = SupervisorView::probe(&paths);
        assert_eq!(v.pid, None);
        assert!(!v.alive);

        // Our own live pid (written with its start-time) → alive.
        let our_pid = std::process::id();
        pid_file::write_pid(&paths.supervisor_pid(), our_pid).unwrap();
        let v = SupervisorView::probe(&paths);
        assert_eq!(v.pid, Some(our_pid));
        assert!(v.alive, "our own recorded pid must read alive");

        // A recorded-but-dead pid (guaranteed-free high value) → orphaned.
        std::fs::write(paths.supervisor_pid(), "2147483646").unwrap();
        let v = SupervisorView::probe(&paths);
        assert_eq!(v.pid, Some(2_147_483_646));
        assert!(!v.alive, "a dead recorded pid must read not-alive");
    }
}
