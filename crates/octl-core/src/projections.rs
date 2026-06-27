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
