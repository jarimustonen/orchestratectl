//! Read-side helpers for projection files.

use std::path::Path;

use crate::atomic::write_json_atomic;
use crate::error::{Error, Result};
use crate::paths::RunPaths;
use crate::schema::{
    Discussion, DiscussionId, Manifest, Node, NodeId, ProposalId, SpinoffProposal,
    SUPPORTED_STATE_SCHEMAS,
};

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::json(path, e))
}

fn read_json_opt<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(|e| Error::json(path, e))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn check_schema(path: &Path, found: u32) -> Result<()> {
    if SUPPORTED_STATE_SCHEMAS.contains(&found) {
        Ok(())
    } else {
        Err(Error::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            found,
            supported: SUPPORTED_STATE_SCHEMAS.to_vec(),
        })
    }
}

/// Reject a projection whose body id (`body`) does not equal the filename key
/// it was read under (`expected`). Both are already-validated id newtypes — a
/// well-formed `nodes/n-0002.json` mis-filed as `nodes/n-0001.json` parses
/// cleanly yet describes a different object, so handing it back would let a
/// later keyed write clobber a third file. `kind` names the projection type so
/// a caller can branch on the [`Error::CorruptProjection`] it produces.
fn check_key(kind: &'static str, expected: &str, body: &str) -> Result<()> {
    if expected == body {
        Ok(())
    } else {
        Err(Error::CorruptProjection {
            kind,
            expected_id: expected.to_string(),
            body_id: body.to_string(),
        })
    }
}

/// Reject a write whose object `run_id` (`body`) does not equal the run the
/// [`RunPaths`] is anchored on (`expected`). Guards every `write_*` helper
/// against stamping a foreign run's id into this run's directory — feasible now
/// that `RunPaths` carries a typed [`crate::RunId`]. `kind` is the write-side
/// discriminator (`"node_run_id"`, etc.).
fn check_run_id(kind: &'static str, expected: &str, body: &str) -> Result<()> {
    check_key(kind, expected, body)
}

/// Read and schema-validate the run manifest. Errors if it is missing.
pub fn read_manifest(paths: &RunPaths) -> Result<Manifest> {
    let p = paths.manifest();
    let m: Manifest = read_json(&p)?;
    check_schema(&p, m.schema_version)?;
    Ok(m)
}

/// Read and schema-validate the run manifest, returning `None` if absent.
pub fn read_manifest_opt(paths: &RunPaths) -> Result<Option<Manifest>> {
    let p = paths.manifest();
    match read_json_opt::<Manifest>(&p)? {
        Some(m) => {
            check_schema(&p, m.schema_version)?;
            Ok(Some(m))
        }
        None => Ok(None),
    }
}

/// Atomically write the run manifest (temp file + rename).
///
/// `pub(crate)`: projection writes belong to the reducer. External callers
/// mutate state through [`crate::events::append_and_apply_event`] so a write
/// can never bypass the event log or the run's `flock`.
pub(crate) fn write_manifest(paths: &RunPaths, m: &Manifest) -> Result<()> {
    check_run_id("manifest_run_id", paths.run_id.as_str(), m.run_id.as_str())?;
    write_json_atomic(&paths.manifest(), m)
}

/// Read and schema-validate one node. Errors if it is missing.
pub fn read_node(paths: &RunPaths, node_id: &NodeId) -> Result<Node> {
    let p = paths.node(node_id);
    let n: Node = read_json(&p)?;
    check_schema(&p, n.schema_version)?;
    check_key("node", node_id.as_str(), n.node_id.as_str())?;
    Ok(n)
}

/// Read and schema-validate one node, returning `None` if absent.
pub fn read_node_opt(paths: &RunPaths, node_id: &NodeId) -> Result<Option<Node>> {
    let p = paths.node(node_id);
    match read_json_opt::<Node>(&p)? {
        Some(n) => {
            check_schema(&p, n.schema_version)?;
            check_key("node", node_id.as_str(), n.node_id.as_str())?;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

/// Atomically write a node's projection file, keyed by its `node_id`.
///
/// Stays `pub` (unlike the other `write_*` helpers) as the sanctioned
/// lock-held composition path for the supervisor batch: the supervisor
/// mirrors per-child report cursors and a child's `supervisor_pid` directly
/// onto the node projection while holding the run's `flock` — fields no
/// event/reducer path manages. Pair it with [`crate::RunLock`].
pub fn write_node(paths: &RunPaths, n: &Node) -> Result<()> {
    check_run_id("node_run_id", paths.run_id.as_str(), n.run_id.as_str())?;
    write_json_atomic(&paths.node(&n.node_id), n)
}

/// Read and schema-validate one discussion. Errors if it is missing.
pub fn read_discussion(paths: &RunPaths, id: &DiscussionId) -> Result<Discussion> {
    let p = paths.discussion(id);
    let d: Discussion = read_json(&p)?;
    check_schema(&p, d.schema_version)?;
    check_key("discussion", id.as_str(), d.discussion_id.as_str())?;
    Ok(d)
}

/// Read and schema-validate one discussion, returning `None` if absent.
pub fn read_discussion_opt(paths: &RunPaths, id: &DiscussionId) -> Result<Option<Discussion>> {
    let p = paths.discussion(id);
    match read_json_opt::<Discussion>(&p)? {
        Some(d) => {
            check_schema(&p, d.schema_version)?;
            check_key("discussion", id.as_str(), d.discussion_id.as_str())?;
            Ok(Some(d))
        }
        None => Ok(None),
    }
}

/// Atomically write a discussion file, keyed by its `discussion_id`.
///
/// `pub(crate)`: see [`write_manifest`].
pub(crate) fn write_discussion(paths: &RunPaths, d: &Discussion) -> Result<()> {
    check_run_id(
        "discussion_run_id",
        paths.run_id.as_str(),
        d.run_id.as_str(),
    )?;
    write_json_atomic(&paths.discussion(&d.discussion_id), d)
}

/// Read and schema-validate one spin-off proposal. Errors if it is missing.
pub fn read_spinoff(paths: &RunPaths, id: &ProposalId) -> Result<SpinoffProposal> {
    let p = paths.spinoff(id);
    let s: SpinoffProposal = read_json(&p)?;
    check_schema(&p, s.schema_version)?;
    check_key("spinoff", id.as_str(), s.proposal_id.as_str())?;
    Ok(s)
}

/// Read and schema-validate one spin-off proposal, returning `None` if absent.
pub fn read_spinoff_opt(paths: &RunPaths, id: &ProposalId) -> Result<Option<SpinoffProposal>> {
    let p = paths.spinoff(id);
    match read_json_opt::<SpinoffProposal>(&p)? {
        Some(s) => {
            check_schema(&p, s.schema_version)?;
            check_key("spinoff", id.as_str(), s.proposal_id.as_str())?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

/// Atomically write a spin-off proposal file, keyed by its `proposal_id`.
///
/// `pub(crate)`: see [`write_manifest`].
pub(crate) fn write_spinoff(paths: &RunPaths, s: &SpinoffProposal) -> Result<()> {
    check_run_id("spinoff_run_id", paths.run_id.as_str(), s.run_id.as_str())?;
    write_json_atomic(&paths.spinoff(&s.proposal_id), s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::STATE_SCHEMA_VERSION;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    /// The run this run-directory belongs to, and a *different* well-formed run
    /// id used to forge the cross-run mismatch the write guards reject.
    const RUN: &str = "01jxsnap000000000000000000";
    const FOREIGN_RUN: &str = "02jxsnap000000000000000000";

    /// A run dir under a fresh tempdir, with the projection subdirectories
    /// created so a hand-written file can be dropped at any key.
    fn setup() -> (TempDir, RunPaths) {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("run");
        let paths = RunPaths::new(&dir, RUN).unwrap();
        std::fs::create_dir_all(paths.nodes_dir()).unwrap();
        std::fs::create_dir_all(paths.discussions_dir()).unwrap();
        std::fs::create_dir_all(paths.spinoffs_dir()).unwrap();
        (tmp, paths)
    }

    fn node_json(node_id: &str, run_id: &str) -> Value {
        json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "node_id": node_id,
            "run_id": run_id,
            "parent_node_id": null,
            "kind": "spinoff",
            "status": "pending",
            "task": null,
            "worktree_path": null,
            "branch": null,
            "tmux_window": null,
            "agent_pid": null,
            "agent_pid_start_time": null,
            "supervisor_pid": null,
            "children": [],
            "started_at": null,
            "updated_at": "2026-06-12T00:00:00Z",
            "last_report": null,
            "last_processed_report_seq_by_child": {}
        })
    }

    fn discussion_json(discussion_id: &str, run_id: &str) -> Value {
        json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "discussion_id": discussion_id,
            "run_id": run_id,
            "node_id": "n-0001",
            "opened_at": "2026-06-12T00:00:00Z",
            "severity": "normal",
            "topic": "fixture",
            "context": null,
            "options": [],
            "status": "open",
            "resolution": null,
            "note": null,
            "resolved_at": null
        })
    }

    fn spinoff_json(proposal_id: &str, run_id: &str) -> Value {
        json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "proposal_id": proposal_id,
            "run_id": run_id,
            "node_id": "n-0001",
            "proposed_at": "2026-06-12T00:00:00Z",
            "proposed_title": "fixture",
            "proposed_kind": "spinoff",
            "rationale": null,
            "status": "proposed",
            "accepted_as_issue_slug": null,
            "rejected_reason": null,
            "resolved_at": null
        })
    }

    fn manifest_json(run_id: &str) -> Value {
        json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "run_id": run_id,
            "kind": "spinoff",
            "lifecycle": "autonomous",
            "title": "fixture",
            "status": "pending",
            "created_at": "2026-06-12T00:00:00Z",
            "updated_at": "2026-06-12T00:00:00Z",
            "source_repo": null,
            "source_branch": null,
            "worktree_root": null,
            "node_count": 0,
            "open_discussions": 0,
            "pending_spinoffs": 0,
            "parent_run_id": null,
            "parent_node_id": null
        })
    }

    fn write_raw(path: &Path, v: &Value) {
        std::fs::write(path, serde_json::to_vec(v).unwrap()).unwrap();
    }

    // --- read side: body id must equal the requested filename key ---------

    #[test]
    fn read_node_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = NodeId::parse_str("n-0001").unwrap();
        // A perfectly valid n-0002 projection, mis-filed at n-0001's path.
        write_raw(&paths.node(&requested), &node_json("n-0002", RUN));
        assert!(matches!(
            read_node(&paths, &requested),
            Err(Error::CorruptProjection { kind: "node", expected_id, body_id })
                if expected_id == "n-0001" && body_id == "n-0002"
        ));
    }

    #[test]
    fn read_node_opt_rejects_body_id_mismatch() {
        // The `_opt` variant is what the reducer and CLI actually call, so the
        // guard must fire there too — a mismatch is an error, not `None`.
        let (_tmp, paths) = setup();
        let requested = NodeId::parse_str("n-0001").unwrap();
        write_raw(&paths.node(&requested), &node_json("n-0002", RUN));
        assert!(matches!(
            read_node_opt(&paths, &requested),
            Err(Error::CorruptProjection { kind: "node", .. })
        ));
    }

    #[test]
    fn read_node_accepts_matching_key() {
        // Guard against a false positive: the well-filed case still reads.
        let (_tmp, paths) = setup();
        let requested = NodeId::parse_str("n-0001").unwrap();
        write_raw(&paths.node(&requested), &node_json("n-0001", RUN));
        let n = read_node(&paths, &requested).unwrap();
        assert_eq!(n.node_id.as_str(), "n-0001");
    }

    #[test]
    fn read_discussion_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = DiscussionId::parse_str("d-01arz3ndektsv4rrffq69g5fav").unwrap();
        write_raw(
            &paths.discussion(&requested),
            &discussion_json("d-01arz3ndektsv4rrffq69g5faw", RUN),
        );
        assert!(matches!(
            read_discussion(&paths, &requested),
            Err(Error::CorruptProjection { kind: "discussion", expected_id, body_id })
                if expected_id == "d-01arz3ndektsv4rrffq69g5fav"
                    && body_id == "d-01arz3ndektsv4rrffq69g5faw"
        ));
    }

    #[test]
    fn read_spinoff_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = ProposalId::parse_str("s-01arz3ndektsv4rrffq69g5fav").unwrap();
        write_raw(
            &paths.spinoff(&requested),
            &spinoff_json("s-01arz3ndektsv4rrffq69g5faw", RUN),
        );
        assert!(matches!(
            read_spinoff(&paths, &requested),
            Err(Error::CorruptProjection { kind: "spinoff", expected_id, body_id })
                if expected_id == "s-01arz3ndektsv4rrffq69g5fav"
                    && body_id == "s-01arz3ndektsv4rrffq69g5faw"
        ));
    }

    // --- write side: object run_id must equal the run's RunPaths.run_id ----

    #[test]
    fn write_node_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let n: Node = serde_json::from_value(node_json("n-0001", FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_node(&paths, &n),
            Err(Error::CorruptProjection { kind: "node_run_id", expected_id, body_id })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        // The forged write never touched disk.
        assert!(!paths.node(&n.node_id).exists());
    }

    #[test]
    fn write_node_accepts_matching_run_id() {
        let (_tmp, paths) = setup();
        let n: Node = serde_json::from_value(node_json("n-0001", RUN)).unwrap();
        write_node(&paths, &n).unwrap();
        assert!(paths.node(&n.node_id).exists());
    }

    #[test]
    fn write_discussion_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let d: Discussion =
            serde_json::from_value(discussion_json("d-01arz3ndektsv4rrffq69g5fav", FOREIGN_RUN))
                .unwrap();
        assert!(matches!(
            write_discussion(&paths, &d),
            Err(Error::CorruptProjection { kind: "discussion_run_id", expected_id, body_id })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
    }

    #[test]
    fn write_spinoff_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let s: SpinoffProposal =
            serde_json::from_value(spinoff_json("s-01arz3ndektsv4rrffq69g5fav", FOREIGN_RUN))
                .unwrap();
        assert!(matches!(
            write_spinoff(&paths, &s),
            Err(Error::CorruptProjection { kind: "spinoff_run_id", expected_id, body_id })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
    }

    #[test]
    fn write_manifest_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let m: Manifest = serde_json::from_value(manifest_json(FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_manifest(&paths, &m),
            Err(Error::CorruptProjection { kind: "manifest_run_id", expected_id, body_id })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        assert!(!paths.manifest().exists());
    }
}
