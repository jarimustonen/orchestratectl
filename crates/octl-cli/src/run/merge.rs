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
//!      the rebase, the cross-worktree `flock`, and the `workmux merge`. v1
//!      deliberately wraps the script rather than re-implementing git
//!      wrappers in Rust (issue §4); v2 can move it into core. The worktree /
//!      window / branch teardown is NOT merge.sh's — it belongs to the
//!      supervisor (state-integrity invariant #5).
//!   2. **Terminal report** — on a clean merge, append a `node.report`
//!      with `via: "explicit-merge"`. That flag is the signal the
//!      supervisor's cleanup gate checks to extend teardown to
//!      *interactive* kinds: a user who runs `run merge` is done with the
//!      review window, so it may close (see supervise/cleanup.rs).
//!   3. **Ensure teardown actually happens.** The supervisor is the canonical
//!      teardown actor, but there is one path where it provably cannot act: a
//!      node the watchdog already terminalized (e.g. an `agent-died` false
//!      positive on a long-lived interactive run) before the merge. The reducer
//!      then drops the late `explicit-merge` report as a dead event, so the
//!      cleanup gate never sees the merge marker and the worktree/branch/window
//!      leak while the envelope says `merged: true` (issues
//!      `merge-skips-teardown`, `agent-died-merge-no-teardown-interactive`). On
//!      exactly that path — detected by re-reading whether the reducer ADOPTED
//!      the report — `run merge` reclaims the worktree + branch itself through
//!      the shared cleanup primitives (the merge landing is authoritative
//!      ground truth), surfacing any incomplete step as an envelope warning,
//!      then closes the tmux window as its last act. Every other path stays the
//!      supervisor's (via [`ensure_report_consumer`]).
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
use octl_core::{
    append_and_apply_event, read_all_events, read_manifest_opt, read_node_opt, Node, RunLock,
};

use crate::error::{CliError, ExitKind};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::dto::SupervisorView;
use crate::run::{from_core, parse_node_id, reattach, require_nonempty, run_paths};
use crate::supervise::cleanup;

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
    /// Outcome of ensuring a live consumer for the terminal report — the
    /// machine-readable companion to any `warnings` entry, so an agent reads a
    /// `state` instead of regex-parsing prose. Present on a real merge, absent
    /// on `--dry-run`. Additive (no `schema_version` bump).
    #[serde(skip_serializing_if = "Option::is_none")]
    supervisor: Option<ConsumerOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

/// Machine-readable result of [`ensure_report_consumer`]: what state the run's
/// per-run supervisor was left in after the terminal report was appended. The
/// non-silent counterpart to the merge `warnings`.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum ConsumerOutcome {
    /// A live supervisor is already consuming the report on its next tick.
    Alive,
    /// The run was already terminal — a supervisor rolled it up and exited, so
    /// nothing remains to consume.
    Terminal,
    /// The run was never supervised (e.g. a skeleton/test run) — there is no
    /// teardown actor to restart, and spawning one would be wrong.
    NotSupervised,
    /// The dead supervisor was restarted; teardown is (re)running. `pid` is the
    /// new supervisor's pid, or `null` when the spawn was not yet confirmed.
    Reattached { pid: Option<u32> },
    /// No live consumer could be ensured; teardown is deferred until the caller
    /// runs `recovery_command`.
    Deferred { recovery_command: String },
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
            supervisor: None,
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

    // Did the reducer ADOPT our terminal report, or drop it as a dead event?
    // The reducer's terminal guard silently no-ops a `node.report` against an
    // already-terminal node and leaves `last_report` untouched (octl-core
    // `reduce_node_report`). So when a prior report terminalized the node before
    // this merge — most commonly a watchdog `agent-died` false positive on a
    // long-lived interactive run — our `via: "explicit-merge"` marker never
    // reaches the projection, `any_node_merged_explicitly` never sees it, and NO
    // supervisor (live or reattached) will ever warrant teardown. That is the
    // silent worktree/branch/window leak of `merge-skips-teardown` /
    // `agent-died-merge-no-teardown-interactive`. Re-read the projection to tell
    // the two apart: `adopted` means a supervisor owns teardown as usual;
    // `!adopted` means `run merge` must reclaim the resources itself, using the
    // merge's own ground truth (merge.sh exited 0 → branch landed in source).
    let adopted = matches!(
        read_node_opt(&paths, &node_id)
            .map_err(from_core)?
            .as_ref()
            .and_then(|n| n.last_report.as_ref())
            .and_then(|r| r.get("via"))
            .and_then(Value::as_str),
        Some("explicit-merge")
    );
    let do_inline_teardown = !adopted;

    // The terminal report is only useful if a live supervisor consumes it: the
    // supervisor is the canonical teardown actor (close tmux window, remove
    // worktree, delete branch) AND the roller-up of `manifest.status` (state
    // integrity invariant #5). If the recorded supervisor has died (e.g. a
    // SIGTERM from a cycling tmux server — the `supervisor-dead-merge-no-teardown`
    // bug), reporting a bare `merged: true` would mislead the caller into
    // telling the user cleanup happened when it never will. So ensure a live
    // consumer before returning, and never return silently on the dead path.
    //
    // EXCEPTION — the swallowed path on an already-terminal run: the reducer
    // dropped our report, so there is nothing for a supervisor to CONSUME, and
    // the run is already rolled up. Reattaching one here would only contend with
    // the inline teardown below — for an autonomous kind the reattached supervisor
    // would read the stale `agent-died` report as a blocked handoff and try to
    // PRESERVE the very branch/worktree this call is reclaiming, writing a
    // misleading `cleanup.branch_preserved` audit event. So skip it and reclaim
    // directly (the run merge review finding on the double-teardown race). A
    // swallowed-but-not-yet-rolled-up run still goes through `ensure_report_consumer`
    // so the run gets rolled up; the inline teardown below is idempotent with it.
    let manifest_terminal = read_manifest_opt(&paths)
        .map_err(from_core)?
        .is_some_and(|m| m.status.is_terminal());
    let (outcome, mut warnings) = if do_inline_teardown && manifest_terminal {
        (ConsumerOutcome::Terminal, args.warnings.to_vec())
    } else {
        ensure_report_consumer(&paths, &run_id, args.warnings)
    };

    // Synchronous teardown on the swallowed-report path. When the reducer dropped
    // our report, the canonical supervisor teardown can never fire, so reclaim the
    // worktree + branch here through the SAME cleanup primitives the supervisor
    // uses (invariant #5's actor is unchanged for every path where it CAN act; this
    // is the one path where it provably cannot). Force teardown is authorized by
    // the merge's own ground truth — `merge.sh` exited 0, so the branch landed in
    // source — exactly the trust the accepted `via: "explicit-merge"` arm rests on
    // (a `--rebase`/squash merge legitimately leaves the branch "ahead" of source,
    // so the source-relative preservation check would false-positive here just as
    // it would there). Teardown runs and completes before this call returns — never
    // after a `merged: true` envelope — and any step that could not finish is
    // surfaced as a `warnings[]` entry rather than a clean close. The tmux window
    // is closed AFTER the envelope is emitted + flushed (below).
    if do_inline_teardown {
        warnings.extend(cleanup::reclaim_merged_worktree_branch(&paths, &node));
    }

    let payload = MergePayload {
        run_id: &run_id,
        node_id: node_id.as_str(),
        branch,
        source: effective_source.as_deref(),
        merged: true,
        report_seq: Some(result.seq),
        supervisor: Some(outcome),
        dry_run: None,
    };
    emit(&payload, args.spec, &warnings)?;

    // Close the merged node's tmux window LAST — after the envelope is emitted AND
    // explicitly flushed. A foreground/interactive `run merge` runs inside the very
    // window it closes, so `tmux kill-window` can SIGHUP this process; the envelope
    // (with any teardown warnings) must be on the wire first, and Rust's block-
    // buffered stdout is not guaranteed flushed on a signal-killed exit. Reclaiming
    // the worktree + branch and reporting them must also complete first (deliverable:
    // a window-kill must never abort the rest of the teardown). Best-effort and
    // idempotent — an already-closed window (a prior run merge, or a supervisor that
    // won the race) is a clean no-op.
    if do_inline_teardown {
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        cleanup::close_merged_node_window(&paths, &node);
    }
    Ok(())
}

/// Guarantee the terminal `node.report` just appended has a live consumer, or
/// surface why not — never silent success. Returns the machine-readable
/// [`ConsumerOutcome`] plus the caller's `base` warnings with (at most) one
/// human-readable entry appended.
///
/// The recovery decision reasons across THREE facts, read together under one
/// shared lock (state-integrity invariant #3): the manifest status, the
/// `supervisor.pid` liveness, and whether a supervisor was EVER started. The
/// discriminator for "reattach or not" is deliberately NOT "is a dead pid file
/// present" — an orphan can also have NO pid file (the `claim_pid_atomic` hint
/// tells an operator to delete a stale `supervisor.pid`, after which a merge
/// would otherwise silently strand the run). The sound rule is:
///
///   reattach  ⟺  no live supervisor  ∧  run not yet terminal  ∧  ever supervised
///
/// where "ever supervised" = a recorded pid (stale file) OR a `supervisor.started`
/// event in the log. A never-materialized skeleton run (e.g. a `--skip-materialize`
/// test run) has neither, so it is left untouched — spawning a supervisor for it
/// would be wrong.
///
/// KNOWN RESIDUAL: a legacy bare-integer `supervisor.pid` whose pid has been
/// recycled by an unrelated live process reads as `alive` (§7.6 identity check
/// cannot fire without a recorded start-time), so this returns `Alive` and skips
/// reattach. Modern pid files (the norm) carry a start-time and are immune.
fn ensure_report_consumer(
    paths: &octl_core::RunPaths,
    run_id: &str,
    base: &[String],
) -> (ConsumerOutcome, Vec<String>) {
    // One shared-locked read of manifest.json + supervisor.pid + events.jsonl:
    // the reattach decision is a multi-projection read, so it must not observe a
    // half-applied set (invariant #3). The event scan is short-circuited when a
    // pid file is already present, so the common (signal-death) path pays only
    // the manifest read + pid probe.
    let probed = RunLock::with_shared_lock(&paths.lock(), || {
        let terminal = read_manifest_opt(paths)?.is_some_and(|m| m.status.is_terminal());
        let live = SupervisorView::probe(paths);
        let ever_supervised = live.pid.is_some()
            || read_all_events(&paths.events())?
                .iter()
                .any(|e| e.kind == "supervisor.started");
        Ok((terminal, live, ever_supervised))
    });

    let (terminal, live, ever_supervised) = match probed {
        Ok(v) => v,
        // A locked read failed (corrupt log / I/O). Don't guess the run's health
        // — the merge itself already landed; surface a deferred-teardown warning
        // so the caller can recover rather than assuming a clean close.
        Err(e) => {
            let mut warnings = base.to_vec();
            warnings.push(format!(
                "could not verify supervisor liveness after merge ({e}); if teardown \
                 does not complete, run `orchestratectl run reattach {run_id}`"
            ));
            return (
                ConsumerOutcome::Deferred {
                    recovery_command: format!("orchestratectl run reattach {run_id}"),
                },
                warnings,
            );
        }
    };

    // A live supervisor will consume the report on its next tick.
    if live.alive {
        return (ConsumerOutcome::Alive, base.to_vec());
    }
    // Already rolled up by a supervisor that has since exited: nothing to consume.
    if terminal {
        return (ConsumerOutcome::Terminal, base.to_vec());
    }
    // Never supervised (skeleton run): no teardown actor to restart.
    if !ever_supervised {
        return (ConsumerOutcome::NotSupervised, base.to_vec());
    }

    // Orphaned: non-terminal, was supervised, no live supervisor remains to
    // consume the terminal report. Restore the invariant by reattaching.
    let who = live.pid.map_or_else(
        || "the supervisor".to_string(),
        |p| format!("supervisor (pid {p})"),
    );
    let mut warnings = base.to_vec();
    let outcome = match reattach::spawn_supervisor(paths, run_id, false, None) {
        Ok(0) => {
            warnings.push(format!(
                "{who} was not running; restarted it to consume the terminal report \
                 and complete teardown (new pid not yet confirmed — check \
                 `orchestratectl run show {run_id}`)"
            ));
            ConsumerOutcome::Reattached { pid: None }
        }
        Ok(new_pid) => {
            warnings.push(format!(
                "{who} was not running; restarted it (pid {new_pid}) to consume the \
                 terminal report and complete teardown"
            ));
            ConsumerOutcome::Reattached { pid: Some(new_pid) }
        }
        // A live supervisor appeared between the probe and the spawn attempt:
        // there is a consumer after all, so this is not a warning condition.
        Err(e) if e.code == "supervisor_already_running" => ConsumerOutcome::Alive,
        Err(e) => {
            let recovery_command = format!("orchestratectl run reattach {run_id}");
            warnings.push(format!(
                "{who} is not running and auto-reattach failed ({}); teardown (tmux \
                 window, worktree, branch) is deferred — run `{recovery_command}` to \
                 complete it",
                e.message
            ));
            ConsumerOutcome::Deferred { recovery_command }
        }
    };
    (outcome, warnings)
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
    let mut report = if let Some(path) = report_file {
        read_report_file(path)?
    } else {
        let summary = source.map_or_else(
            || format!("merged {branch} via run merge"),
            |src| format!("merged {branch} into {src} via run merge"),
        );
        json!({ "success": true, "summary": summary })
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
