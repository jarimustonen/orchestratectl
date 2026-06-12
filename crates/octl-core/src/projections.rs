//! Read-side helpers for projection files.

use std::path::Path;

use crate::atomic::write_json_atomic;
use crate::error::{Error, Result};
use crate::paths::RunPaths;
use crate::schema::{Discussion, Manifest, Node, SpinoffProposal, SUPPORTED_STATE_SCHEMAS};

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

pub fn read_manifest(paths: &RunPaths) -> Result<Manifest> {
    let p = paths.manifest();
    let m: Manifest = read_json(&p)?;
    check_schema(&p, m.schema_version)?;
    Ok(m)
}

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

pub fn write_manifest(paths: &RunPaths, m: &Manifest) -> Result<()> {
    write_json_atomic(&paths.manifest(), m)
}

pub fn read_node(paths: &RunPaths, node_id: &str) -> Result<Node> {
    let p = paths.node(node_id);
    let n: Node = read_json(&p)?;
    check_schema(&p, n.schema_version)?;
    Ok(n)
}

pub fn read_node_opt(paths: &RunPaths, node_id: &str) -> Result<Option<Node>> {
    let p = paths.node(node_id);
    match read_json_opt::<Node>(&p)? {
        Some(n) => {
            check_schema(&p, n.schema_version)?;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

pub fn write_node(paths: &RunPaths, n: &Node) -> Result<()> {
    write_json_atomic(&paths.node(&n.node_id), n)
}

pub fn read_discussion(paths: &RunPaths, id: &str) -> Result<Discussion> {
    let p = paths.discussion(id);
    let d: Discussion = read_json(&p)?;
    check_schema(&p, d.schema_version)?;
    Ok(d)
}

pub fn read_discussion_opt(paths: &RunPaths, id: &str) -> Result<Option<Discussion>> {
    let p = paths.discussion(id);
    match read_json_opt::<Discussion>(&p)? {
        Some(d) => {
            check_schema(&p, d.schema_version)?;
            Ok(Some(d))
        }
        None => Ok(None),
    }
}

pub fn write_discussion(paths: &RunPaths, d: &Discussion) -> Result<()> {
    write_json_atomic(&paths.discussion(&d.discussion_id), d)
}

pub fn read_spinoff(paths: &RunPaths, id: &str) -> Result<SpinoffProposal> {
    let p = paths.spinoff(id);
    let s: SpinoffProposal = read_json(&p)?;
    check_schema(&p, s.schema_version)?;
    Ok(s)
}

pub fn read_spinoff_opt(paths: &RunPaths, id: &str) -> Result<Option<SpinoffProposal>> {
    let p = paths.spinoff(id);
    match read_json_opt::<SpinoffProposal>(&p)? {
        Some(s) => {
            check_schema(&p, s.schema_version)?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

pub fn write_spinoff(paths: &RunPaths, s: &SpinoffProposal) -> Result<()> {
    write_json_atomic(&paths.spinoff(&s.proposal_id), s)
}
