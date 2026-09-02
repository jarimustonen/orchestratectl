//! Read-side helpers for projection files.
//!
//! Each `read_*` here reads exactly one file and is coherent on its own (atomic
//! rename — see [`crate::atomic`]). A caller that reads **several** files as one
//! logical view (e.g. `manifest.json` together with the `nodes/` projection
//! set, whose denormalized counters
//! the reducer updates in the same locked mutation) must wrap the whole scan in
//! [`crate::RunLock::with_shared_lock`] (`LOCK_SH`). That excludes the reducer's
//! exclusive lock for the scan's duration, so the reader observes one committed
//! snapshot rather than a half-applied update (design.md §4). The lock is
//! released before the result is serialized.

use std::path::{Path, PathBuf};

use crate::atomic::write_json_atomic;
use crate::error::{Error, Result};
use crate::paths::{reject_symlink, RunPaths};
use crate::schema::{Manifest, Node, NodeId, RunId, SUPPORTED_STATE_SCHEMAS};

/// Resolve a projection file path while rejecting a symlinked run root, the
/// symlinked subdir, or a symlinked file before the caller opens it — so a
/// tampered run-tree component cannot redirect a read or write outside the run
/// directory. `dir_name` names the containing subdir (for [`Error::SymlinkSubdir`])
/// and `file_kind` names the projection type (for [`Error::SymlinkStateFile`]);
/// both checks run after the run root is guarded. Best-effort containment with a
/// check-then-open TOCTOU gap — see [`reject_symlink`].
fn checked_file(
    paths: &RunPaths,
    subdir: PathBuf,
    dir_name: &'static str,
    file: PathBuf,
    file_kind: &'static str,
) -> Result<PathBuf> {
    paths.guard_root()?;
    reject_symlink(&subdir, || Error::SymlinkSubdir {
        name: dir_name,
        path: subdir.clone(),
    })?;
    reject_symlink(&file, || Error::SymlinkStateFile {
        name: file_kind,
        path: file.clone(),
    })?;
    Ok(file)
}

/// `manifest.json` path, guarding the run root and the manifest file itself.
fn checked_manifest(paths: &RunPaths) -> Result<PathBuf> {
    paths.guard_root()?;
    let p = paths.manifest();
    reject_symlink(&p, || Error::SymlinkStateFile {
        name: "manifest",
        path: p.clone(),
    })?;
    Ok(p)
}

/// `nodes/<id>.json` path with run-root, `nodes/`, and file symlink guards.
fn checked_node(paths: &RunPaths, id: &NodeId) -> Result<PathBuf> {
    checked_file(paths, paths.nodes_dir(), "nodes", paths.node(id), "node")
}

/// Read `path` into bytes with `O_NOFOLLOW`: a projection file replaced by a
/// symlink fails the open (`ELOOP`) rather than redirecting the read. This is
/// the file-level TOCTOU backstop to the `reject_symlink` check the `checked_*`
/// resolvers run before calling in here — projection *writes* go via temp-file +
/// rename (never opening the leaf), so the read is the projection's only
/// follow-through-a-symlink surface. See [`crate::paths::nofollow`].
fn read_nofollow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    crate::paths::nofollow(&mut opts);
    let mut f = opts.open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_nofollow(path).map_err(|e| Error::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::json(path, e))
}

fn read_json_opt<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match read_nofollow(path) {
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

/// The manifest's denormalized counters, recomputed from projection state.
///
/// Returned by [`derive_counters`] as a single snapshot of the `nodes/`
/// directory.
pub(crate) struct DerivedCounters {
    /// Number of node projection files.
    pub node_count: u32,
}

/// Recompute the manifest's denormalized counters directly from the projection
/// directories, so they are a pure function of projection state rather than an
/// incrementally-maintained delta.
///
/// This is the heart of the counter-desync fix (issue
/// `manifest-counter-desync`): [`crate::events::advance_applied_seq`] calls this
/// whenever it advances the `applied_seq` watermark, so the counters persisted
/// alongside the watermark always equal a fresh count of the projection state
/// as it stands *after* an event's projection writes are committed. Deriving the
/// counts removes any delta: drift is impossible because nothing is ever
/// incremented.
///
/// Counting is best-effort under corruption: a directory that does not exist
/// counts as empty, and a projection file that fails to read or parse is
/// skipped rather than bricking every future append (`doctor` surfaces such
/// anomalies). Only regular `*.json` files whose stem is a well-formed
/// projection id are counted, so an in-flight atomic write (a hidden
/// `.<name>.tmp.<pid>.<n>` tempfile) is never miscounted.
pub(crate) fn derive_counters(paths: &RunPaths) -> Result<DerivedCounters> {
    Ok(DerivedCounters {
        node_count: count_node_files(paths)?,
    })
}

/// Open `dir` for a counting walk: a missing directory yields `None` (count 0);
/// a real `read_dir` failure propagates. Pairs with [`projection_id_stem`].
fn open_projection_dir(dir: &Path) -> Result<Option<std::fs::ReadDir>> {
    match std::fs::read_dir(dir) {
        Ok(e) => Ok(Some(e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(dir, e)),
    }
}

/// The id-stem of a projection slot: `Some(stem)` for a regular `*.json` file,
/// `None` for directories, non-`json` entries, and the hidden tempfiles atomic
/// writes leave mid-rename. The caller decides whether `stem` is a valid id.
fn projection_id_stem(ent: &std::fs::DirEntry) -> Option<String> {
    if !ent.file_type().is_ok_and(|t| t.is_file()) {
        return None;
    }
    let path = ent.path();
    if path.extension().and_then(|s| s.to_str()) != Some("json") {
        return None;
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

/// Count node projection files: every regular `nodes/<node-id>.json` whose stem
/// is a well-formed [`NodeId`]. A node file's mere existence means the node was
/// created, so this needs no content read.
fn count_node_files(paths: &RunPaths) -> Result<u32> {
    let dir = paths.nodes_dir();
    let Some(entries) = open_projection_dir(&dir)? else {
        return Ok(0);
    };
    let mut n: u32 = 0;
    for ent in entries {
        let ent = ent.map_err(|e| Error::io(&dir, e))?;
        if let Some(stem) = projection_id_stem(&ent) {
            if NodeId::parse_str(&stem).is_ok() {
                n = n.saturating_add(1);
            }
        }
    }
    Ok(n)
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
            "parent_run_id": null,
            "parent_node_id": null
        })
    }

    fn write_raw(path: &Path, v: &Value) {
        std::fs::write(path, serde_json::to_vec(v).unwrap()).unwrap();
    }

    // --- derived counters --------------------------------------------------

    #[test]
    fn derive_counters_counts_projection_state_and_ignores_junk() {
        let (_tmp, paths) = setup();
        // Two nodes.
        write_raw(
            &paths.node(&NodeId::parse_str("n-0001").unwrap()),
            &node_json("n-0001", RUN),
        );
        write_raw(
            &paths.node(&NodeId::parse_str("n-0002").unwrap()),
            &node_json("n-0002", RUN),
        );

        // Junk that must be ignored: a non-`json` file, a `json` file whose stem
        // is not a valid id, and a hidden tempfile mimicking an in-flight atomic
        // write.
        std::fs::write(paths.nodes_dir().join("README.txt"), b"x").unwrap();
        std::fs::write(paths.nodes_dir().join("not-an-id.json"), b"{}").unwrap();
        std::fs::write(paths.nodes_dir().join(".n-0003.json.tmp.123.0"), b"{}").unwrap();

        let c = derive_counters(&paths).unwrap();
        assert_eq!(c.node_count, 2);
    }

    #[test]
    fn derive_counters_missing_dirs_are_zero() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(&dir, RUN).unwrap();
        // No nodes/ subdirectory exists.
        let c = derive_counters(&paths).unwrap();
        assert_eq!(c.node_count, 0);
    }

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
            Err(Error::SymlinkStateFile { name: "node", .. })
        ));
    }

    /// The `O_NOFOLLOW` backstop, isolated from the `symlink_metadata` check
    /// the `checked_*` resolvers run first: calling the leaf reader directly on
    /// a symlinked projection must fail the *open* with `ELOOP` rather than
    /// following it. This is the half of the TOCTOU window that survives a leaf
    /// swapped *after* the `symlink_metadata` check but *before* the open.
    #[cfg(unix)]
    #[test]
    fn read_json_refuses_to_follow_a_symlinked_projection() {
        use std::os::unix::fs::symlink;
        let (tmp, _paths) = setup();
        let target = tmp.path().join("evil-node.json");
        write_raw(&target, &node_json("n-0001", RUN));
        let link = tmp.path().join("link-node.json");
        symlink(&target, &link).unwrap();
        let err = read_json::<Node>(&link).expect_err("must refuse a symlinked projection");
        match err {
            Error::Io { source, .. } => assert_eq!(
                source.raw_os_error(),
                Some(libc::ELOOP),
                "O_NOFOLLOW open of a symlink must report ELOOP, got {source:?}"
            ),
            other => panic!("expected Error::Io(ELOOP), got {other:?}"),
        }
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
    fn read_node_re_guards_a_run_root_swapped_after_construction() {
        // The access-time guard must catch a root that becomes a symlink AFTER
        // the (now-checked) constructor ran — the long-lived-handle case. Build
        // a clean RunPaths, then swap its root dir for a symlink to an outside
        // dir that holds an otherwise-valid node, and confirm the read refuses
        // to follow it.
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("run");
        let paths = RunPaths::new(&root, RUN).unwrap();
        let id = NodeId::parse_str("n-0001").unwrap();
        // Outside target with a real nodes/ and a valid node behind it.
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(outside.join("nodes")).unwrap();
        write_raw(
            &outside.join("nodes/n-0001.json"),
            &node_json("n-0001", RUN),
        );
        // Swap the real run dir for a symlink to `outside`.
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::remove_dir(&root).unwrap();
        symlink(&outside, &root).unwrap();
        assert!(matches!(
            read_node(&paths, &id),
            Err(Error::SymlinkRunDir { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn from_validated_rejects_a_symlinked_run_root_at_construction() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            RunPaths::from_validated(link, RunId::parse_str(RUN).unwrap()),
            Err(Error::SymlinkRunDir { .. })
        ));
    }

    // --- manifest + write-side file symlink coverage ----------------------

    #[cfg(unix)]
    #[test]
    fn read_manifest_rejects_symlinked_manifest_file() {
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let target = tmp.path().join("evil-manifest.json");
        write_raw(&target, &manifest_json(RUN));
        symlink(&target, paths.manifest()).unwrap();
        assert!(matches!(
            read_manifest(&paths),
            Err(Error::SymlinkStateFile {
                name: "manifest",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn write_node_rejects_symlinked_node_file() {
        // Write side: a symlinked target file is refused before the atomic
        // temp+rename runs, so the forged write never reaches the link target.
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let id = NodeId::parse_str("n-0001").unwrap();
        let target = tmp.path().join("evil-node.json");
        symlink(&target, paths.node(&id)).unwrap();
        let n: Node = serde_json::from_value(node_json("n-0001", RUN)).unwrap();
        assert!(matches!(
            write_node(&paths, &n),
            Err(Error::SymlinkStateFile { name: "node", .. })
        ));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn read_node_rejects_dangling_symlinked_file() {
        // A symlink whose target does not exist is still a symlink — it must be
        // rejected as corruption, not treated as an absent file (`None`).
        use std::os::unix::fs::symlink;
        let (tmp, paths) = setup();
        let id = NodeId::parse_str("n-0001").unwrap();
        symlink(tmp.path().join("does-not-exist.json"), paths.node(&id)).unwrap();
        assert!(matches!(
            read_node_opt(&paths, &id),
            Err(Error::SymlinkStateFile { name: "node", .. })
        ));
    }
}
