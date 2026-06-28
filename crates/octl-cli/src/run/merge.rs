//! `run merge` — own the full merge lifecycle of one worktree run.
//!
//! Closes the spawn → work → merge → cleanup loop end-to-end inside
//! orchestratectl (issue `bundle-worktree-merge`). Before this verb the
//! merge half lived in the homebase `/worktree-merge` bash skill, which
//! had no knowledge of the run: it merged, but never submitted a terminal
//! `node.report`, so the supervisor kept polling and (for interactive
//! kinds) never tore the window down. `run merge` does both halves in one
//! call:
//!
//!   1. **Merge mechanics** — shell out to the bundled `merge.sh`
//!      (embedded below, materialized to a temp file at runtime). It owns
//!      the rebase, the cross-worktree `flock`, the `workmux merge`, and
//!      the proven detached worktree/window/branch teardown. v1
//!      deliberately wraps the script rather than re-implementing git
//!      wrappers in Rust (issue §4); v2 can move it into core.
//!   2. **Terminal report** — on a clean merge, append a `node.report`
//!      with `via: "explicit-merge"`. That flag is the signal the
//!      supervisor's cleanup gate checks to extend teardown to
//!      *interactive* kinds: a user who runs `run merge` is done with the
//!      review window, so it may close (see supervise/cleanup.rs).
//!
//! On a merge failure (conflicts, dirty tree, lock timeout) the report is
//! NOT submitted — the node stays live so the agent can recover (e.g.
//! `/complex-rebase`) and re-run `run merge`.

use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::report::validate_report_payload;
use octl_core::{append_and_apply_event, read_manifest_opt, read_node_opt, Node};

use crate::error::{CliError, ExitKind};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, require_nonempty, run_paths};

/// The bundled merge backend, embedded at compile time so the binary is
/// self-contained (the homebase `merge.sh` is sunset). Materialized to a
/// temp file and executed per invocation. Tests override the resolved
/// script via `OCTL_MERGE_SH`, mirroring `spawn.rs`'s `OCTL_CREATE_SH`.
const MERGE_SH: &str = include_str!("../../scripts/merge.sh");

/// Default reporting node for a single-worker run. Every worktree kind
/// `run merge` targets has exactly one node.
const DEFAULT_NODE_ID: &str = "n-0001";

/// Cap on `--report-file` size, mirroring `node report`'s 1 MiB bound.
const MAX_REPORT_BYTES: u64 = 1024 * 1024;

pub struct Args<'a> {
    pub run_id: String,
    /// Override the merge target branch. Falls back to the manifest's
    /// `source_branch`, then to merge.sh's own main/master auto-detection.
    pub source: Option<String>,
    /// Reporting node id; defaults to `n-0001`.
    pub node_id: Option<String>,
    /// Optional §7.3 report payload to submit on a clean merge. When set,
    /// `run merge` stamps it with `via: "explicit-merge"` and submits it —
    /// so an autonomous kind can carry its rich `discussion_items` /
    /// `spinoff_proposals` / `wrap_up_recommendations` in the SAME call
    /// that merges. When absent, a minimal `{success, summary, via}` report
    /// is submitted (enough for a simple spinoff).
    pub report_file: Option<PathBuf>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct MergePayload<'a> {
    run_id: &'a str,
    node_id: &'a str,
    branch: &'a str,
    /// The resolved merge target, or `null` when left to merge.sh's
    /// main/master auto-detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    merged: bool,
    /// `node.report` seq, when a terminal report was appended.
    #[serde(skip_serializing_if = "Option::is_none")]
    report_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = args.run_id.clone();
    let node_id = parse_node_id(args.node_id.as_deref().unwrap_or(DEFAULT_NODE_ID))?;
    let source = match args.source {
        Some(s) => Some(require_nonempty(&s, "source")?),
        None => None,
    };

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

    let manifest = read_manifest_opt(&paths)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id)
        })?;

    let node = read_node_opt(&paths, &node_id)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user(
                "node_not_found",
                format!("no node {node_id} in run {run_id}"),
            )
            .with_invalid_value(node_id.as_str())
        })?;

    // The worktree directory is the cwd the merge backend needs: it derives
    // the branch and the source-side worktree from `git` run there. A node
    // with no materialized worktree (a driver node) cannot be merged.
    let worktree_path = node.worktree_path.as_deref().ok_or_else(|| {
        CliError::user(
            "no_worktree",
            format!("node {node_id} has no worktree to merge (driver node?)"),
        )
        .with_invalid_value(node_id.as_str())
    })?;
    let branch = branch_for(&node).ok_or_else(|| {
        CliError::user(
            "no_branch",
            format!("node {node_id} has no branch recorded; cannot merge"),
        )
        .with_invalid_value(node_id.as_str())
    })?;

    // Resolve the merge target: explicit `--source` wins, else the
    // manifest's source_branch (the integration branch for an orchestrated
    // child, `main` for a code worktree), else None → merge.sh detects
    // main/master itself.
    let effective_source = source.clone().or_else(|| manifest.source_branch.clone());

    // Build the terminal report up front — BEFORE the merge — so a malformed
    // `--report-file` is rejected without having already merged. The report
    // is only submitted after a clean merge; here we just validate its shape
    // and stamp the `via: "explicit-merge"` marker.
    let report = build_report(
        args.report_file.as_deref(),
        branch,
        effective_source.as_deref(),
    )?;

    if args.dry_run {
        let payload = MergePayload {
            run_id: &run_id,
            node_id: node_id.as_str(),
            branch,
            source: effective_source.as_deref(),
            merged: false,
            report_seq: None,
            dry_run: Some(true),
        };
        return emit(&payload, args.spec, args.warnings);
    }

    // Run the merge. A non-zero exit (conflict, dirty tree, lock timeout)
    // surfaces as a CliError and the report is NOT submitted — the node
    // stays live for the agent to recover and retry.
    run_merge_sh(
        Path::new(worktree_path),
        branch,
        effective_source.as_deref(),
    )?;

    // Merge succeeded: submit the terminal report (built above, stamped with
    // `via: "explicit-merge"`) so the supervisor's cleanup gate extends
    // teardown to interactive kinds and any rich `discussion_items` /
    // `spinoff_proposals` reach the parent.
    //
    // Idempotent: a retried `run merge` (e.g. the report append failed but
    // the merge already landed) re-uses the same key and returns the prior
    // seq instead of double-appending. The merge itself is also a clean
    // no-op on retry (the branch is already merged, worktree may be gone).
    let idem_key = format!("explicit-merge:{run_id}:{node_id}");
    let result = append_and_apply_event(
        &paths,
        "node.report",
        Some(&node_id),
        Some(&idem_key),
        report,
    )
    .map_err(from_core)?;

    let payload = MergePayload {
        run_id: &run_id,
        node_id: node_id.as_str(),
        branch,
        source: effective_source.as_deref(),
        merged: true,
        report_seq: Some(result.seq),
        dry_run: None,
    };
    emit(&payload, args.spec, args.warnings)
}

/// The branch a node works on. Prefers the explicit `branch` field; a
/// well-formed worktree node always has it.
fn branch_for(node: &Node) -> Option<&str> {
    node.branch.as_deref().filter(|s| !s.is_empty())
}

/// Build (and validate) the terminal §7.3 report to submit on a clean merge.
///
/// With `report_file`, read the agent's payload (so an autonomous kind can
/// carry its `discussion_items` / `spinoff_proposals` /
/// `wrap_up_recommendations` in the same call) and stamp it with the
/// `via: "explicit-merge"` marker, overriding any caller-set `via`. Without
/// one, synthesize a minimal `{success, summary, via}` report — enough for a
/// simple spinoff. Either way the result is validated against the §7.3 schema
/// before it can reach the event log.
fn build_report(
    report_file: Option<&Path>,
    branch: &str,
    source: Option<&str>,
) -> Result<Value, CliError> {
    let mut report = match report_file {
        Some(path) => read_report_file(path)?,
        None => {
            let summary = match source {
                Some(src) => format!("merged {branch} into {src} via run merge"),
                None => format!("merged {branch} via run merge"),
            };
            json!({ "success": true, "summary": summary })
        }
    };
    // `run merge` owns the marker: stamp it regardless of what the file held.
    let obj = report.as_object_mut().ok_or_else(|| {
        CliError::user(
            "report_not_object",
            "--report-file payload must be a JSON object",
        )
    })?;
    obj.insert(
        "via".to_string(),
        Value::String("explicit-merge".to_string()),
    );

    validate_report_payload(&report)
        .map_err(|e| CliError::user("schema_violation", e.to_string()))?;
    Ok(report)
}

/// Read and JSON-parse a `--report-file`, enforcing the size cap during the
/// read (TOCTOU-safe, mirroring `node report`'s `read_capped`).
fn read_report_file(path: &Path) -> Result<Value, CliError> {
    let mut f = std::fs::File::open(path).map_err(|e| {
        CliError::user(
            "report_file_unreadable",
            format!("open {}: {}", path.display(), e),
        )
        .with_invalid_value(path.display().to_string())
    })?;
    let mut buf = Vec::new();
    std::io::Read::by_ref(&mut f)
        .take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| {
            CliError::user(
                "report_file_unreadable",
                format!("read {}: {}", path.display(), e),
            )
            .with_invalid_value(path.display().to_string())
        })?;
    if buf.len() as u64 > MAX_REPORT_BYTES {
        return Err(CliError::user(
            "report_file_too_large",
            format!("--report-file exceeds maximum of {MAX_REPORT_BYTES} bytes"),
        )
        .with_invalid_value(path.display().to_string()));
    }
    serde_json::from_slice(&buf).map_err(|e| {
        CliError::user(
            "report_file_invalid_json",
            format!("parse {}: {}", path.display(), e),
        )
        .with_invalid_value(path.display().to_string())
    })
}

/// Resolve the merge backend: `OCTL_MERGE_SH` override (tests) or the
/// embedded script materialized to a temp file with the exec bit set.
/// Returns the temp-file guard so it lives until the command has run.
fn materialize_merge_sh() -> Result<MergeScript, CliError> {
    if let Ok(path) = std::env::var("OCTL_MERGE_SH") {
        return Ok(MergeScript::External(path.into()));
    }
    let mut tmp = tempfile::Builder::new()
        .prefix("orchestratectl-merge-")
        .suffix(".sh")
        .tempfile()
        .map_err(|e| {
            CliError::system("tempfile_failed", format!("create merge.sh tempfile: {e}"))
        })?;
    tmp.write_all(MERGE_SH.as_bytes())
        .map_err(|e| CliError::system("write_failed", format!("write merge.sh tempfile: {e}")))?;
    tmp.flush()
        .map_err(|e| CliError::system("write_failed", format!("flush merge.sh tempfile: {e}")))?;
    let perms = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(tmp.path(), perms)
        .map_err(|e| CliError::system("chmod_failed", format!("chmod merge.sh tempfile: {e}")))?;
    Ok(MergeScript::Temp(tmp))
}

/// Where the materialized merge backend lives — an external override path
/// or an owned temp file that must outlive the command invocation.
enum MergeScript {
    External(std::path::PathBuf),
    Temp(tempfile::NamedTempFile),
}

impl MergeScript {
    fn path(&self) -> &Path {
        match self {
            MergeScript::External(p) => p.as_path(),
            MergeScript::Temp(t) => t.path(),
        }
    }
}

/// Invoke the merge backend from inside `worktree_path`, inheriting the
/// environment (notably `$TMUX`/`$TMUX_PANE`, which the backend uses to
/// close the agent's window). On a non-zero exit, the captured stderr
/// becomes the error message and the report is skipped by the caller.
fn run_merge_sh(worktree_path: &Path, branch: &str, source: Option<&str>) -> Result<(), CliError> {
    let script = materialize_merge_sh()?;
    let mut cmd = Command::new(script.path());
    cmd.current_dir(worktree_path);
    if let Some(src) = source {
        cmd.arg("--target").arg(src);
    }
    cmd.arg(branch);

    let output = cmd.output().map_err(|e| {
        CliError::system(
            "merge_spawn_failed",
            format!("invoke merge.sh ({}): {}", script.path().display(), e),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // merge.sh refuses on preconditions (on main, dirty tree, same
        // branch, lock timeout) and fails on a rebase conflict from
        // `workmux merge`. Both are user-actionable: the agent recovers
        // (commit / resolve / `/complex-rebase`) and retries `run merge`.
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            stdout.trim()
        } else {
            detail
        };
        return Err(CliError {
            kind: ExitKind::User,
            code: "merge_failed".to_string(),
            message: format!(
                "merge.sh exited {} merging {branch}: {detail}",
                output.status.code().unwrap_or(-1)
            ),
            invalid_value: Some(branch.to_string()),
            expected: None,
        });
    }
    Ok(())
}

fn emit(
    payload: &MergePayload<'_>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:     {}", payload.run_id);
            println!("node-id:    {}", payload.node_id);
            println!("branch:     {}", payload.branch);
            match payload.source {
                Some(s) => println!("source:     {s}"),
                None => println!("source:     (auto-detect main/master)"),
            }
            if payload.dry_run == Some(true) {
                println!("note:       --dry-run (no merge, no report)");
            } else {
                println!("merged:     {}", payload.merged);
                if let Some(seq) = payload.report_seq {
                    println!("report_seq: {seq}");
                }
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
