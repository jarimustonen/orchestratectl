//! Read-side helpers for projection files.

use std::path::{Path, PathBuf};

use crate::atomic::write_json_atomic;
use crate::error::{Error, Result};
use crate::paths::{reject_symlink, RunPaths};
use crate::schema::{
    Discussion, DiscussionId, Manifest, Node, NodeId, ProposalId, RunId, SpinoffProposal,
    SUPPORTED_STATE_SCHEMAS,
};

/// Resolve a projection file path while rejecting a symlinked run root, the
/// symlinked subdir, or a symlinked file before the caller opens it — so a
/// tampered run-tree component cannot redirect a read or write outside the run
/// directory. `subdir`/`name` identify the containing projection directory and
/// `file` is the resolved per-id path; both checks run after the run root is
/// guarded. Best-effort containment with a check-then-open TOCTOU gap — see
/// [`reject_symlink`].
fn checked_file(
    paths: &RunPaths,
    subdir: PathBuf,
    name: &'static str,
    file: PathBuf,
) -> Result<PathBuf> {
    paths.guard_root()?;
    reject_symlink(&subdir, || Error::SymlinkSubdir {
        name,
        path: subdir.clone(),
    })?;
    reject_symlink(&file, || Error::SymlinkProjectionFile { path: file.clone() })?;
    Ok(file)
}

/// `manifest.json` path, guarding the run root and the manifest file itself.
fn checked_manifest(paths: &RunPaths) -> Result<PathBuf> {
    paths.guard_root()?;
    let p = paths.manifest();
    reject_symlink(&p, || Error::SymlinkProjectionFile { path: p.clone() })?;
    Ok(p)
}

/// `nodes/<id>.json` path with run-root, `nodes/`, and file symlink guards.
fn checked_node(paths: &RunPaths, id: &NodeId) -> Result<PathBuf> {
    checked_file(paths, paths.nodes_dir(), "nodes", paths.node(id))
}

/// `discussions/<id>.json` path with run-root, `discussions/`, and file guards.
fn checked_discussion(paths: &RunPaths, id: &DiscussionId) -> Result<PathBuf> {
    checked_file(paths, paths.discussions_dir(), "discussions", paths.discussion(id))
}

/// `spinoffs/<id>.json` path with run-root, `spinoffs/`, and file guards.
fn checked_spinoff(paths: &RunPaths, id: &ProposalId) -> Result<PathBuf> {
    checked_file(paths, paths.spinoffs_dir(), "spinoffs", paths.spinoff(id))
}

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
/// a caller can branch on the [`Error::CorruptProjection`] it produces; `path`
/// localizes the offending file.
fn check_key(path: &Path, kind: &'static str, expected: &str, body: &str) -> Result<()> {
    if expected == body {
        Ok(())
    } else {
        Err(Error::CorruptProjection {
            kind,
            path: path.to_path_buf(),
            expected_id: expected.to_string(),
            body_id: body.to_string(),
        })
    }
}

/// Reject a projection whose object `run_id` (`body`) does not equal the run
/// the [`RunPaths`] is anchored on (`expected`). Fires on both sides: every
/// `read_*` rejects a file that belongs to a foreign run before handing it
/// back, and every `write_*` refuses to stamp a foreign run's id into this
/// run's directory — feasible now that `RunPaths` carries a typed
/// [`crate::RunId`]. Takes `&RunId` (not `&str`) so a caller cannot transpose
/// the arguments or pass an unrelated id. `kind` is the run-id discriminator
/// (`"node_run_id"`, etc.); `path` localizes the file.
fn check_run_id(path: &Path, kind: &'static str, expected: &RunId, body: &RunId) -> Result<()> {
    if expected == body {
        Ok(())
    } else {
        Err(Error::CorruptProjection {
            kind,
            path: path.to_path_buf(),
            expected_id: expected.to_string(),
            body_id: body.to_string(),
        })
    }
}

/// Read and schema-validate the run manifest. Errors if it is missing.
pub fn read_manifest(paths: &RunPaths) -> Result<Manifest> {
    let p = checked_manifest(paths)?;
    let m: Manifest = read_json(&p)?;
    check_schema(&p, m.schema_version)?;
    check_run_id(&p, "manifest_run_id", &paths.run_id, &m.run_id)?;
    Ok(m)
}

/// Read and schema-validate the run manifest, returning `None` if absent.
///
/// A present manifest whose `run_id` belongs to a foreign run is an error, not
/// `None`: `_opt` means "missing file is fine", not "corrupt file is absent".
pub fn read_manifest_opt(paths: &RunPaths) -> Result<Option<Manifest>> {
    let p = checked_manifest(paths)?;
    match read_json_opt::<Manifest>(&p)? {
        Some(m) => {
            check_schema(&p, m.schema_version)?;
            check_run_id(&p, "manifest_run_id", &paths.run_id, &m.run_id)?;
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
    let p = checked_manifest(paths)?;
    check_run_id(&p, "manifest_run_id", &paths.run_id, &m.run_id)?;
    write_json_atomic(&p, m)
}

/// Read and schema-validate one node. Errors if it is missing.
pub fn read_node(paths: &RunPaths, node_id: &NodeId) -> Result<Node> {
    let p = checked_node(paths, node_id)?;
    let n: Node = read_json(&p)?;
    check_schema(&p, n.schema_version)?;
    check_key(&p, "node", node_id.as_str(), n.node_id.as_str())?;
    check_run_id(&p, "node_run_id", &paths.run_id, &n.run_id)?;
    Ok(n)
}

/// Read and schema-validate one node, returning `None` if absent.
///
/// A present node whose body id or `run_id` does not match where it lives is an
/// error, not `None`: `_opt` covers a missing file, not a corrupt one.
pub fn read_node_opt(paths: &RunPaths, node_id: &NodeId) -> Result<Option<Node>> {
    let p = checked_node(paths, node_id)?;
    match read_json_opt::<Node>(&p)? {
        Some(n) => {
            check_schema(&p, n.schema_version)?;
            check_key(&p, "node", node_id.as_str(), n.node_id.as_str())?;
            check_run_id(&p, "node_run_id", &paths.run_id, &n.run_id)?;
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
    let p = checked_node(paths, &n.node_id)?;
    check_run_id(&p, "node_run_id", &paths.run_id, &n.run_id)?;
    write_json_atomic(&p, n)
}

/// Read and schema-validate one discussion. Errors if it is missing.
pub fn read_discussion(paths: &RunPaths, id: &DiscussionId) -> Result<Discussion> {
    let p = checked_discussion(paths, id)?;
    let d: Discussion = read_json(&p)?;
    check_schema(&p, d.schema_version)?;
    check_key(&p, "discussion", id.as_str(), d.discussion_id.as_str())?;
    check_run_id(&p, "discussion_run_id", &paths.run_id, &d.run_id)?;
    Ok(d)
}

/// Read and schema-validate one discussion, returning `None` if absent.
///
/// A present discussion whose body id or `run_id` does not match where it lives
/// is an error, not `None`: `_opt` covers a missing file, not a corrupt one.
pub fn read_discussion_opt(paths: &RunPaths, id: &DiscussionId) -> Result<Option<Discussion>> {
    let p = checked_discussion(paths, id)?;
    match read_json_opt::<Discussion>(&p)? {
        Some(d) => {
            check_schema(&p, d.schema_version)?;
            check_key(&p, "discussion", id.as_str(), d.discussion_id.as_str())?;
            check_run_id(&p, "discussion_run_id", &paths.run_id, &d.run_id)?;
            Ok(Some(d))
        }
        None => Ok(None),
    }
}

/// Atomically write a discussion file, keyed by its `discussion_id`.
///
/// `pub(crate)`: see [`write_manifest`].
pub(crate) fn write_discussion(paths: &RunPaths, d: &Discussion) -> Result<()> {
    let p = checked_discussion(paths, &d.discussion_id)?;
    check_run_id(&p, "discussion_run_id", &paths.run_id, &d.run_id)?;
    write_json_atomic(&p, d)
}

/// Read and schema-validate one spin-off proposal. Errors if it is missing.
pub fn read_spinoff(paths: &RunPaths, id: &ProposalId) -> Result<SpinoffProposal> {
    let p = checked_spinoff(paths, id)?;
    let s: SpinoffProposal = read_json(&p)?;
    check_schema(&p, s.schema_version)?;
    check_key(&p, "spinoff", id.as_str(), s.proposal_id.as_str())?;
    check_run_id(&p, "spinoff_run_id", &paths.run_id, &s.run_id)?;
    Ok(s)
}

/// Read and schema-validate one spin-off proposal, returning `None` if absent.
///
/// A present proposal whose body id or `run_id` does not match where it lives
/// is an error, not `None`: `_opt` covers a missing file, not a corrupt one.
pub fn read_spinoff_opt(paths: &RunPaths, id: &ProposalId) -> Result<Option<SpinoffProposal>> {
    let p = checked_spinoff(paths, id)?;
    match read_json_opt::<SpinoffProposal>(&p)? {
        Some(s) => {
            check_schema(&p, s.schema_version)?;
            check_key(&p, "spinoff", id.as_str(), s.proposal_id.as_str())?;
            check_run_id(&p, "spinoff_run_id", &paths.run_id, &s.run_id)?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

/// Atomically write a spin-off proposal file, keyed by its `proposal_id`.
///
/// `pub(crate)`: see [`write_manifest`].
pub(crate) fn write_spinoff(paths: &RunPaths, s: &SpinoffProposal) -> Result<()> {
    let p = checked_spinoff(paths, &s.proposal_id)?;
    check_run_id(&p, "spinoff_run_id", &paths.run_id, &s.run_id)?;
    write_json_atomic(&p, s)
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

    // Two valid 26-char Crockford ULID bodies differing in the last char —
    // used as the "requested key" vs "mis-filed body" pair for discussions and
    // spinoffs (the prefix is supplied per type).
    const ULID_A: &str = "01arz3ndektsv4rrffq69g5fav";
    const ULID_B: &str = "01arz3ndektsv4rrffq69g5faw";

    // --- read side: body id must equal the requested filename key ---------

    #[test]
    fn read_node_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = NodeId::parse_str("n-0001").unwrap();
        // A perfectly valid n-0002 projection, mis-filed at n-0001's path.
        let p = paths.node(&requested);
        write_raw(&p, &node_json("n-0002", RUN));
        assert!(matches!(
            read_node(&paths, &requested),
            Err(Error::CorruptProjection { kind: "node", path, expected_id, body_id })
                if path == p && expected_id == "n-0001" && body_id == "n-0002"
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
    fn read_node_rejects_foreign_run_id() {
        // Correct filename key, but the body belongs to another run — the file
        // was copied/restored from a foreign run directory.
        let (_tmp, paths) = setup();
        let requested = NodeId::parse_str("n-0001").unwrap();
        write_raw(&paths.node(&requested), &node_json("n-0001", FOREIGN_RUN));
        assert!(matches!(
            read_node(&paths, &requested),
            Err(Error::CorruptProjection { kind: "node_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
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
        let requested = DiscussionId::parse_str(&format!("d-{ULID_A}")).unwrap();
        write_raw(
            &paths.discussion(&requested),
            &discussion_json(&format!("d-{ULID_B}"), RUN),
        );
        assert!(matches!(
            read_discussion(&paths, &requested),
            Err(Error::CorruptProjection { kind: "discussion", expected_id, body_id, .. })
                if expected_id == format!("d-{ULID_A}") && body_id == format!("d-{ULID_B}")
        ));
    }

    #[test]
    fn read_discussion_opt_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = DiscussionId::parse_str(&format!("d-{ULID_A}")).unwrap();
        write_raw(
            &paths.discussion(&requested),
            &discussion_json(&format!("d-{ULID_B}"), RUN),
        );
        assert!(matches!(
            read_discussion_opt(&paths, &requested),
            Err(Error::CorruptProjection {
                kind: "discussion",
                ..
            })
        ));
    }

    #[test]
    fn read_discussion_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let requested = DiscussionId::parse_str(&format!("d-{ULID_A}")).unwrap();
        write_raw(
            &paths.discussion(&requested),
            &discussion_json(&format!("d-{ULID_A}"), FOREIGN_RUN),
        );
        assert!(matches!(
            read_discussion(&paths, &requested),
            Err(Error::CorruptProjection { kind: "discussion_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
    }

    #[test]
    fn read_spinoff_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = ProposalId::parse_str(&format!("s-{ULID_A}")).unwrap();
        write_raw(
            &paths.spinoff(&requested),
            &spinoff_json(&format!("s-{ULID_B}"), RUN),
        );
        assert!(matches!(
            read_spinoff(&paths, &requested),
            Err(Error::CorruptProjection { kind: "spinoff", expected_id, body_id, .. })
                if expected_id == format!("s-{ULID_A}") && body_id == format!("s-{ULID_B}")
        ));
    }

    #[test]
    fn read_spinoff_opt_rejects_body_id_mismatch() {
        let (_tmp, paths) = setup();
        let requested = ProposalId::parse_str(&format!("s-{ULID_A}")).unwrap();
        write_raw(
            &paths.spinoff(&requested),
            &spinoff_json(&format!("s-{ULID_B}"), RUN),
        );
        assert!(matches!(
            read_spinoff_opt(&paths, &requested),
            Err(Error::CorruptProjection {
                kind: "spinoff",
                ..
            })
        ));
    }

    #[test]
    fn read_spinoff_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let requested = ProposalId::parse_str(&format!("s-{ULID_A}")).unwrap();
        write_raw(
            &paths.spinoff(&requested),
            &spinoff_json(&format!("s-{ULID_A}"), FOREIGN_RUN),
        );
        assert!(matches!(
            read_spinoff(&paths, &requested),
            Err(Error::CorruptProjection { kind: "spinoff_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
    }

    #[test]
    fn read_manifest_rejects_foreign_run_id() {
        // The manifest is keyed by its directory, so a foreign-run manifest
        // restored into this run's dir is the same class of corruption.
        let (_tmp, paths) = setup();
        write_raw(&paths.manifest(), &manifest_json(FOREIGN_RUN));
        assert!(matches!(
            read_manifest(&paths),
            Err(Error::CorruptProjection { kind: "manifest_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        // `_opt` must reject it too, not paper over it as `None`.
        assert!(matches!(
            read_manifest_opt(&paths),
            Err(Error::CorruptProjection {
                kind: "manifest_run_id",
                ..
            })
        ));
    }

    #[test]
    fn read_manifest_accepts_matching_run_id() {
        let (_tmp, paths) = setup();
        write_raw(&paths.manifest(), &manifest_json(RUN));
        assert_eq!(read_manifest(&paths).unwrap().run_id.as_str(), RUN);
    }

    // --- write side: object run_id must equal the run's RunPaths.run_id ----

    #[test]
    fn write_node_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let n: Node = serde_json::from_value(node_json("n-0001", FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_node(&paths, &n),
            Err(Error::CorruptProjection { kind: "node_run_id", expected_id, body_id, .. })
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
            serde_json::from_value(discussion_json(&format!("d-{ULID_A}"), FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_discussion(&paths, &d),
            Err(Error::CorruptProjection { kind: "discussion_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        assert!(!paths.discussion(&d.discussion_id).exists());
    }

    #[test]
    fn write_discussion_accepts_matching_run_id() {
        let (_tmp, paths) = setup();
        let d: Discussion =
            serde_json::from_value(discussion_json(&format!("d-{ULID_A}"), RUN)).unwrap();
        write_discussion(&paths, &d).unwrap();
        assert!(paths.discussion(&d.discussion_id).exists());
    }

    #[test]
    fn write_spinoff_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let s: SpinoffProposal =
            serde_json::from_value(spinoff_json(&format!("s-{ULID_A}"), FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_spinoff(&paths, &s),
            Err(Error::CorruptProjection { kind: "spinoff_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        assert!(!paths.spinoff(&s.proposal_id).exists());
    }

    #[test]
    fn write_spinoff_accepts_matching_run_id() {
        let (_tmp, paths) = setup();
        let s: SpinoffProposal =
            serde_json::from_value(spinoff_json(&format!("s-{ULID_A}"), RUN)).unwrap();
        write_spinoff(&paths, &s).unwrap();
        assert!(paths.spinoff(&s.proposal_id).exists());
    }

    #[test]
    fn write_manifest_rejects_foreign_run_id() {
        let (_tmp, paths) = setup();
        let m: Manifest = serde_json::from_value(manifest_json(FOREIGN_RUN)).unwrap();
        assert!(matches!(
            write_manifest(&paths, &m),
            Err(Error::CorruptProjection { kind: "manifest_run_id", expected_id, body_id, .. })
                if expected_id == RUN && body_id == FOREIGN_RUN
        ));
        assert!(!paths.manifest().exists());
    }

    #[test]
    fn write_manifest_accepts_matching_run_id() {
        let (_tmp, paths) = setup();
        let m: Manifest = serde_json::from_value(manifest_json(RUN)).unwrap();
        write_manifest(&paths, &m).unwrap();
        assert!(paths.manifest().exists());
    }

    // --- symlink containment: a replaced subdir or file is refused ---------
    //
    // Each test stores a *valid* projection behind the symlink so the rejection
    // can only come from the symlink guard, never from a parse/key/run-id check
    // downstream.

    #[cfg(unix)]
    #[test]
    fn read_node_rejects_symlinked_nodes_dir() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir(paths.nodes_dir()).unwrap();
        symlink(&outside, paths.nodes_dir()).unwrap();
        let id = NodeId::parse_str("n-0001").unwrap();
        write_raw(&outside.join("n-0001.json"), &node_json("n-0001", RUN));
        assert!(matches!(
            read_node(&paths, &id),
            Err(Error::SymlinkSubdir { name: "nodes", .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn read_node_rejects_symlinked_node_file() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let id = NodeId::parse_str("n-0001").unwrap();
        let target = tmp.path().join("evil-node.json");
        write_raw(&target, &node_json("n-0001", RUN));
        symlink(&target, paths.node(&id)).unwrap();
        assert!(matches!(
            read_node(&paths, &id),
            Err(Error::SymlinkProjectionFile { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn read_discussion_rejects_symlinked_discussions_dir() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir(paths.discussions_dir()).unwrap();
        symlink(&outside, paths.discussions_dir()).unwrap();
        let id = DiscussionId::parse_str(&format!("d-{ULID_A}")).unwrap();
        write_raw(
            &outside.join(format!("d-{ULID_A}.json")),
            &discussion_json(&format!("d-{ULID_A}"), RUN),
        );
        assert!(matches!(
            read_discussion(&paths, &id),
            Err(Error::SymlinkSubdir {
                name: "discussions",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn read_discussion_rejects_symlinked_discussion_file() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let id = DiscussionId::parse_str(&format!("d-{ULID_A}")).unwrap();
        let target = tmp.path().join("evil-discussion.json");
        write_raw(&target, &discussion_json(&format!("d-{ULID_A}"), RUN));
        symlink(&target, paths.discussion(&id)).unwrap();
        assert!(matches!(
            read_discussion(&paths, &id),
            Err(Error::SymlinkProjectionFile { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn read_spinoff_rejects_symlinked_spinoffs_dir() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir(paths.spinoffs_dir()).unwrap();
        symlink(&outside, paths.spinoffs_dir()).unwrap();
        let id = ProposalId::parse_str(&format!("s-{ULID_A}")).unwrap();
        write_raw(
            &outside.join(format!("s-{ULID_A}.json")),
            &spinoff_json(&format!("s-{ULID_A}"), RUN),
        );
        assert!(matches!(
            read_spinoff(&paths, &id),
            Err(Error::SymlinkSubdir {
                name: "spinoffs",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn read_spinoff_rejects_symlinked_spinoff_file() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let id = ProposalId::parse_str(&format!("s-{ULID_A}")).unwrap();
        let target = tmp.path().join("evil-spinoff.json");
        write_raw(&target, &spinoff_json(&format!("s-{ULID_A}"), RUN));
        symlink(&target, paths.spinoff(&id)).unwrap();
        assert!(matches!(
            read_spinoff(&paths, &id),
            Err(Error::SymlinkProjectionFile { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn write_node_rejects_symlinked_nodes_dir() {
        // The write side is guarded too: a symlinked subdir would otherwise
        // land the atomic temp+rename outside the run tree.
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir(paths.nodes_dir()).unwrap();
        symlink(&outside, paths.nodes_dir()).unwrap();
        let n: Node = serde_json::from_value(node_json("n-0001", RUN)).unwrap();
        assert!(matches!(
            write_node(&paths, &n),
            Err(Error::SymlinkSubdir { name: "nodes", .. })
        ));
        // The forged write never reached the symlink target.
        assert!(!outside.join("n-0001.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn read_node_rejects_symlinked_run_root() {
        // A symlinked run root must be refused even when every subdir and file
        // beneath it is a perfectly ordinary file — `from_validated` skips the
        // construction-time check, so the read path re-guards the root.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real-run");
        let real_paths = RunPaths::new(&real, RUN).unwrap();
        std::fs::create_dir_all(real_paths.nodes_dir()).unwrap();
        let id = NodeId::parse_str("n-0001").unwrap();
        write_raw(&real_paths.node(&id), &node_json("n-0001", RUN));
        let link = tmp.path().join("link-run");
        symlink(&real, &link).unwrap();
        let linked = RunPaths::from_validated(link, RunId::parse_str(RUN).unwrap());
        assert!(matches!(
            read_node(&linked, &id),
            Err(Error::SymlinkRunDir { .. })
        ));
    }
}
