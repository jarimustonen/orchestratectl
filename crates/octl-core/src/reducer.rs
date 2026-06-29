//! Event → projection reducer (design.md §1.4).
//!
//! Each event mutates zero or more projection files. Unknown kinds are
//! ignored for forward compatibility. The reducer expects to run under the
//! per-run `flock`.
//!
//! Idempotency contract (per design.md §7.3 at-least-once delivery):
//! `*.created` reducers short-circuit when their projection file already
//! exists; status/resolution reducers are no-ops once the terminal state
//! has been reached. Replaying the same event stream against existing
//! projections is therefore a clean no-op-or-apply.
//!
//! This idempotence is load-bearing for the `applied_seq` watermark
//! (append-then-apply atomicity; see [`crate::schema::Manifest::applied_seq`]
//! and [`crate::events::append_and_apply_event`]). Of the two options the spec
//! offered — make the reducer idempotent, OR have the writer skip events
//! already reflected in the projection — we chose **idempotent reducer**: the
//! existence/terminal guards already present here mean the catch-up replay can
//! re-fold *any* tail event (one whose projection landed before a crash, or one
//! whose projection did not) with the same no-op-or-apply outcome, so the
//! writer needs no per-event "already applied?" probe. The watermark advances
//! only after an event's projections are fsynced, so it can lag the projections
//! but never lead them.
//!
//! ## Manifest counters are derived, not folded
//!
//! The reducers here deliberately do **not** touch the manifest's denormalized
//! counters (`node_count`, `open_discussions`, `pending_spinoffs`). Those are
//! recomputed from the projection directories by
//! [`derive_counters`](crate::projections), invoked from
//! [`advance_applied_seq`](crate::events) at the
//! watermark advance. An earlier design incremented/decremented them inside
//! these reducers, but a crash between a projection write and the follow-on
//! `manifest.json` write could permanently desync them: the replay re-folded
//! the event, hit the `*.created`/terminal idempotency guard above, and skipped
//! the counter mutation that never actually landed. Deriving the counts makes
//! drift impossible — there is no delta to lose. See issue
//! `manifest-counter-desync`. A count-affecting reducer still emits its manifest
//! op to refresh `updated_at`; the counter fields it carries are overwritten by
//! the derive step.
//!
//! Because a count-affecting event rewrites a projection file *and* the
//! manifest counter under the same exclusive `flock`, a reader that scans both
//! together must hold the shared `flock` (`LOCK_SH`) for the whole scan or it
//! could see the projection change without the matching counter (or vice
//! versa). See [`crate::projections`] and design.md §4.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::paths::RunPaths;
use crate::projections::{
    read_discussion_opt, read_manifest_opt, read_node_opt, read_spinoff_opt, write_discussion,
    write_manifest, write_node, write_spinoff,
};
use crate::schema::{
    ChildRef, Discussion, DiscussionId, DiscussionStatus, Event, IdValidationError, Kind,
    Lifecycle, Manifest, Node, NodeId, ProposalId, RunId, SpinoffProposal, SpinoffStatus, Status,
    TmuxIdentity, STATE_SCHEMA_VERSION,
};

/// Map an id-validation failure on an event-sourced id to a [`CorruptEventLog`]
/// error. An id that fails to parse here came off `events.jsonl` (or a forged
/// event), so the log — not the caller — is the corrupt party.
///
/// [`CorruptEventLog`]: Error::CorruptEventLog
fn corrupt_id(events_path: &Path, ev: &Event, e: &IdValidationError) -> Error {
    Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason: format!("event seq={} kind={}: {e}", ev.seq, ev.kind),
    }
}

/// Parse an optional `RunId` from event-data field `field`: missing/null →
/// `None`; a JSON string → validated `Some(RunId)`; a malformed id or a
/// non-string value → [`Error::CorruptEventLog`].
fn opt_run_id(events_path: &Path, ev: &Event, d: &Value, field: &str) -> Result<Option<RunId>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => RunId::parse_str(s)
            .map(Some)
            .map_err(|e| corrupt_id(events_path, ev, &e)),
        Some(_) => Err(Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} `{field}` must be a JSON string or null",
                ev.seq, ev.kind
            ),
        }),
    }
}

/// Parse an optional `NodeId` from event-data field `field`. See [`opt_run_id`].
fn opt_node_id(events_path: &Path, ev: &Event, d: &Value, field: &str) -> Result<Option<NodeId>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => NodeId::parse_str(s)
            .map(Some)
            .map_err(|e| corrupt_id(events_path, ev, &e)),
        Some(_) => Err(Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} `{field}` must be a JSON string or null",
                ev.seq, ev.kind
            ),
        }),
    }
}

/// Resolve a required `NodeId` from event-data field `field`, falling back to
/// the envelope's top-level `node_id`. Used where the node reference may appear
/// either in `data` or on the envelope (discussion/spinoff `node_id`).
fn want_node_id_with_fallback(
    events_path: &Path,
    ev: &Event,
    d: &Value,
    field: &str,
) -> Result<NodeId> {
    let s = d
        .get(field)
        .and_then(Value::as_str)
        .or(ev.node_id.as_ref().map(NodeId::as_str))
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!("event seq={} kind={} missing `{field}`", ev.seq, ev.kind),
        })?;
    NodeId::parse_str(s).map_err(|e| corrupt_id(events_path, ev, &e))
}

fn data_kind(v: &Value) -> Option<Kind> {
    serde_json::from_value(v.clone()).ok()
}

fn data_status(v: &Value) -> Option<Status> {
    serde_json::from_value(v.clone()).ok()
}

fn require_status(ev: &Event, path: PathBuf) -> Result<Status> {
    data_status(ev.data.get("status").unwrap_or(&Value::Null)).ok_or_else(|| {
        Error::CorruptEventLog {
            path,
            reason: format!("{} missing/invalid `status`", ev.kind),
        }
    })
}

fn want_str<'a>(events_path: &Path, ev: &Event, d: &'a Value, field: &str) -> Result<&'a str> {
    d.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} missing `{field}` string field",
                ev.seq, ev.kind
            ),
        })
}

/// Read an optional string field with strict typing: missing/null → `None`,
/// JSON string → `Some(s)`, anything else → `CorruptEventLog`. Prevents
/// the reducer from silently dropping non-string payload values.
fn optional_str(events_path: &Path, ev: &Event, d: &Value, field: &str) -> Result<Option<String>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} `{field}` must be a JSON string or null",
                ev.seq, ev.kind
            ),
        }),
    }
}

/// Read an optional boolean field with strict typing: missing/null → `None`,
/// JSON bool → `Some(b)`, anything else → `CorruptEventLog`. Mirrors
/// [`optional_str`] / [`optional_i32`]; prevents a non-boolean `success` /
/// `cancelled` from being silently coerced to `false` and bypassing the
/// success-XOR-cancelled invariant.
fn optional_bool(events_path: &Path, ev: &Event, d: &Value, field: &str) -> Result<Option<bool>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} `{field}` must be a JSON boolean or null",
                ev.seq, ev.kind
            ),
        }),
    }
}

fn optional_i32(d: &Value, field: &str, events_path: &Path, ev: &Event) -> Result<Option<i32>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let raw = v.as_i64().ok_or_else(|| Error::CorruptEventLog {
                path: events_path.to_path_buf(),
                reason: format!(
                    "event seq={} kind={} `{field}` must be integer",
                    ev.seq, ev.kind
                ),
            })?;
            i32::try_from(raw)
                .map(Some)
                .map_err(|_| Error::CorruptEventLog {
                    path: events_path.to_path_buf(),
                    reason: format!(
                        "event seq={} kind={} `{field}` out of i32 range: {raw}",
                        ev.seq, ev.kind
                    ),
                })
        }
    }
}

fn optional_ts(
    d: &Value,
    field: &str,
    events_path: &Path,
    ev: &Event,
) -> Result<Option<DateTime<Utc>>> {
    match d.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => DateTime::parse_from_rfc3339(s)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|_| Error::CorruptEventLog {
                path: events_path.to_path_buf(),
                reason: format!(
                    "event seq={} kind={} `{field}` not RFC3339",
                    ev.seq, ev.kind
                ),
            }),
        Some(_) => Err(Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: format!(
                "event seq={} kind={} `{field}` must be RFC3339 string or null",
                ev.seq, ev.kind
            ),
        }),
    }
}

/// A projection write planned by [`reduce_event_to_ops`] and performed by
/// [`commit_ops`].
///
/// Splitting the reducer into a pure *plan* phase (compute these ops from the
/// current projection state, validating as it goes) and a *commit* phase
/// (write them) means a single branch per kind implements both the pre-append
/// validation gate and the post-append apply — there is no validate/apply
/// mirror to drift out of lockstep, and the projection state is read once
/// rather than twice.
pub(crate) enum ProjectionOp {
    /// Write the run manifest.
    Manifest(Manifest),
    /// Write a node projection.
    Node(Node),
    /// Write a discussion projection.
    Discussion(Discussion),
    /// Write a spinoff-proposal projection.
    Spinoff(SpinoffProposal),
}

/// Commit a planned batch of projection writes, in order.
///
/// Caller must hold the run's [`crate::lock::RunLock`]. Pairs with
/// [`reduce_event_to_ops`]: the ops were computed against the same locked
/// state, and nothing mutates the projections between the plan and this commit
/// (in the append path only `events.jsonl` is written in between), so the
/// planned writes are still valid.
pub(crate) fn commit_ops(paths: &RunPaths, ops: Vec<ProjectionOp>) -> Result<()> {
    for op in ops {
        match op {
            ProjectionOp::Manifest(m) => write_manifest(paths, &m)?,
            ProjectionOp::Node(n) => write_node(paths, &n)?,
            ProjectionOp::Discussion(d) => write_discussion(paths, &d)?,
            ProjectionOp::Spinoff(s) => write_spinoff(paths, &s)?,
        }
    }
    Ok(())
}

/// Plan the projection writes one event implies, *without* performing them.
///
/// This is the single source of truth for both validation and application: it
/// reads the current projection state, enforces every event-payload invariant
/// (returning [`Error::CorruptEventLog`] for a malformed or cross-run event),
/// and returns the exact [`ProjectionOp`]s to commit (empty for a no-op or an
/// unknown `kind`). Because it never writes, it is also the transactional gate
/// run *before* the durable append in
/// [`crate::events::append_and_apply_unlocked`]: a reducer-rejected event is
/// caught here and never reaches `events.jsonl`, so a later replay /
/// `rebuild_projections` can't trip over a poison line. The state-dependent
/// no-op guards live here too (a settled node/run/discussion swallows a late
/// or even malformed event as a clean no-op rather than erroring).
///
/// Caller must hold the run's [`crate::lock::RunLock`].
pub(crate) fn reduce_event_to_ops(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    // An event whose envelope `run_id` doesn't match the run we're folding it
    // into means the log was copied/misrouted — folding it would silently
    // cross-contaminate projections. Reject before planning anything.
    if ev.run_id != paths.run_id {
        return Err(Error::CorruptEventLog {
            path: paths.events(),
            reason: format!(
                "event seq={} envelope run_id {:?} does not match run {:?}",
                ev.seq,
                ev.run_id.as_str(),
                paths.run_id.as_str()
            ),
        });
    }
    // Each event kind is listed explicitly as documentation of the known set;
    // `supervisor.exited` and the `_` fallthrough share a body intentionally.
    #[allow(clippy::match_same_arms)]
    match ev.kind.as_str() {
        "run.created" => reduce_run_created(paths, ev),
        "run.status" => reduce_run_status(paths, ev),
        "node.created" => reduce_node_created(paths, ev),
        "node.status" => reduce_node_status(paths, ev),
        "node.report" => reduce_node_report(paths, ev),
        "discussion.opened" => reduce_discussion_opened(paths, ev),
        "discussion.resolved" => reduce_discussion_resolved(paths, ev),
        "spinoff.proposed" => reduce_spinoff_proposed(paths, ev),
        "spinoff.approved" => reduce_spinoff_approved(paths, ev),
        "spinoff.rejected" => reduce_spinoff_rejected(paths, ev),
        "child.spawned" => reduce_child_spawned(paths, ev),
        "supervisor.attached" => reduce_supervisor_attached(paths, ev),
        "supervisor.cursor_advanced" => reduce_supervisor_cursor_advanced(paths, ev),
        "supervisor.exited" => Ok(vec![]),
        // Append-only audit records from `/orchestrate` (decision log +
        // pakkopysäytys). They mutate no projection — the event log is their
        // canonical home — so they fold to a clean no-op. Listed explicitly
        // (rather than relying on the `_` fallthrough) so the append path's
        // transactional gate runs the same no-op plan for them and the intent
        // is documented at the match site. They are NOT `node.report`, so the
        // supervisor never mistakes them for a terminal signal.
        "orchestrator.decision" | "discuss.critical" => Ok(vec![]),
        // Best-effort teardown audit records from the supervisor's cleanup
        // path. Each mutates no projection — the event log is their only home —
        // so they fold to a clean no-op. Listed explicitly so the append path's
        // transactional gate runs the same no-op plan and the intent is
        // documented here.
        //   - `cleanup.window_missing`: the node's tmux window could not be
        //     located to close it (typically a manually-resolved rebase renamed
        //     the window — issue `worktree-merge-orphans-tmux-window`).
        //   - `cleanup.worktree_missing`: the worktree dir was already gone at
        //     teardown (e.g. removed manually), so nothing to `worktree remove`.
        //   - `cleanup.branch_remove_failed`: `git branch -D` refused (e.g.
        //     unexpected unmerged commits); the run completes anyway (issue
        //     `supervisor-worktree-remove-no-force`).
        //   - `cleanup.session_killed`: the run's managed `--headless` tmux
        //     session was torn down once its last managed window was gone, so an
        //     empty session is not left behind (issue
        //     `headless-tmux-session-not-torn-down`).
        //   - `cleanup.session_retained`: the same teardown was skipped because a
        //     human had attached to the session — never yanked out from under
        //     them.
        "cleanup.window_missing"
        | "cleanup.worktree_missing"
        | "cleanup.branch_remove_failed"
        | "cleanup.session_killed"
        | "cleanup.session_retained" => Ok(vec![]),
        _ => Ok(vec![]),
    }
}

/// The projection file [`commit_ops`] writes for `op`, keyed exactly as the
/// `write_*` helpers key it internally. Shared by [`plan_projections`] (which
/// reports the path) and conceptually by [`commit_ops`] (which writes it), so
/// the enumerated path list can never name a different file than the one the
/// reducer actually fsyncs.
fn op_path(paths: &RunPaths, op: &ProjectionOp) -> PathBuf {
    match op {
        ProjectionOp::Manifest(_) => paths.manifest(),
        ProjectionOp::Node(n) => paths.node(&n.node_id),
        ProjectionOp::Discussion(d) => paths.discussion(&d.discussion_id),
        ProjectionOp::Spinoff(s) => paths.spinoff(&s.proposal_id),
    }
}

/// Enumerate the projection files the reducer would write for `event`, in the
/// order `commit_ops` would write them, *without* performing any write.
///
/// This is the single source of truth that ends the CLI/reducer divergence the
/// `projected-paths-into-reducer` issue describes: rather than a hand-maintained
/// list in `octl-cli` that drifts whenever a new projection is added, both the
/// reducer and a caller's preflight (`event create --dry-run`) read the *same*
/// `reduce_event_to_ops` plan. This function maps that plan to file paths;
/// `apply_event` commits it. A new projection added to a reducer arm is
/// therefore reflected here automatically.
///
/// Because it runs the real reducer plan against current projection state, the
/// result is exact, not a guess: a state-dependent no-op (a settled node, an
/// already-created projection, a terminal-guarded transition) yields an empty
/// list — precisely the files `apply_event` would touch, which is none. A
/// malformed-payload event surfaces the same [`Error::CorruptEventLog`] the
/// real apply would, so a dry-run preflight cannot report success for an event
/// the write path would reject.
///
/// Caller should hold the run's [`crate::lock::RunLock`] for a snapshot
/// consistent with a concurrent reducer; a lock-free read is best-effort.
pub fn plan_projections(paths: &RunPaths, event: &Event) -> Result<Vec<PathBuf>> {
    let ops = reduce_event_to_ops(paths, event)?;
    Ok(ops.iter().map(|op| op_path(paths, op)).collect())
}

/// Apply one event to projections: plan via [`reduce_event_to_ops`], then
/// [`commit_ops`]. No-op for unknown `kind`. Caller must hold the run's
/// [`crate::lock::RunLock`].
///
/// Shares the one [`reduce_event_to_ops`] plan with [`plan_projections`]: the
/// paths that function reports are exactly the files this one fsyncs, because
/// both consume the same `ProjectionOp` vector (this commits it; that maps it to
/// paths via [`op_path`]).
///
/// `pub(crate)`: applying an event in isolation (without the matching
/// `events.jsonl` append) is an internal building block used by `cancel` (to
/// re-fold a crash-stranded event) and a future `rebuild_projections_from_events`.
/// External callers mutate state through
/// [`crate::events::append_and_apply_event`] so the log and projections can
/// never diverge.
pub(crate) fn apply_event(paths: &RunPaths, ev: &Event) -> Result<()> {
    let ops = reduce_event_to_ops(paths, ev)?;
    commit_ops(paths, ops)
}

/// Validate an event WITHOUT writing anything — [`reduce_event_to_ops`] with
/// the planned writes discarded. Returns `Err` in exactly the cases
/// [`apply_event`] would (they share the one plan), so a dry-run check can
/// never drift from the apply.
///
/// `#[cfg(test)]`: the append path validates by inspecting
/// `reduce_event_to_ops` directly (it needs the planned ops anyway), so this
/// discard-the-ops wrapper exists only for the reducer's agreement tests.
#[cfg(test)]
pub(crate) fn validate_event(paths: &RunPaths, ev: &Event) -> Result<()> {
    reduce_event_to_ops(paths, ev).map(|_| ())
}

/// The envelope `node_id` that a `node.*` event must carry, with the same
/// `CorruptEventLog` message `apply_*` produces. Shared by validate/apply so
/// the missing-id check can't drift between them.
fn require_envelope_node_id(events_path: &Path, ev: &Event) -> Result<NodeId> {
    ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason: format!(
            "event seq={} kind={} missing top-level `node_id`",
            ev.seq, ev.kind
        ),
    })
}

fn reduce_run_created(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    // Idempotent: a replayed `run.created` against an existing manifest is a
    // no-op (but validates that `run_id` matches; otherwise the event log
    // is being applied to the wrong run).
    if let Some(existing) = read_manifest_opt(paths)? {
        if existing.run_id != ev.run_id {
            return Err(Error::CorruptEventLog {
                path: paths.manifest(),
                reason: format!(
                    "run.created run_id={} conflicts with existing manifest run_id={}",
                    ev.run_id, existing.run_id
                ),
            });
        }
        return Ok(vec![]);
    }
    let events_path = paths.events();
    let d = &ev.data;
    let kind =
        data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: "run.created missing/invalid `kind`".into(),
        })?;
    let lifecycle: Lifecycle = serde_json::from_value(
        d.get("lifecycle").cloned().unwrap_or(Value::Null),
    )
    .map_err(|_| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: "run.created missing/invalid `lifecycle`".into(),
    })?;
    let title = want_str(&events_path, ev, d, "title")?.to_string();
    let m = Manifest {
        schema_version: STATE_SCHEMA_VERSION,
        // Created at the watermark floor; the append path advances it to this
        // event's `seq` (after the manifest is fsynced) in `advance_applied_seq`.
        applied_seq: 0,
        // `run_id == paths.run_id` was verified at `reduce_event_to_ops` entry.
        run_id: paths.run_id.clone(),
        kind,
        lifecycle,
        title,
        status: Status::Pending,
        created_at: ev.ts,
        updated_at: ev.ts,
        source_repo: d
            .get("source_repo")
            .and_then(Value::as_str)
            .map(str::to_string),
        source_branch: d
            .get("source_branch")
            .and_then(Value::as_str)
            .map(str::to_string),
        worktree_root: d
            .get("worktree_root")
            .and_then(Value::as_str)
            .map(str::to_string),
        managed_tmux_session: d
            .get("managed_tmux_session")
            .and_then(Value::as_str)
            .map(str::to_string),
        node_count: 0,
        open_discussions: 0,
        pending_spinoffs: 0,
        parent_run_id: opt_run_id(&events_path, ev, d, "parent_run_id")?,
        parent_node_id: opt_node_id(&events_path, ev, d, "parent_node_id")?,
    };
    Ok(vec![ProjectionOp::Manifest(m)])
}

fn reduce_run_status(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let mut m = match read_manifest_opt(paths)? {
        Some(m) => m,
        None => return Ok(vec![]),
    };
    let new_status = require_status(ev, paths.events())?;
    // Terminal-state guard: a settled run never transitions again (e.g. a
    // late `run.status running` after a cancel). See run-cli-read/handoff.md D5.
    if m.status.is_terminal() {
        trace_terminal_noop(ev, m.status, new_status);
        return Ok(vec![]);
    }
    if m.status == new_status {
        return Ok(vec![]);
    }
    m.status = new_status;
    m.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Manifest(m)])
}

/// Reconstruct the fully-qualified tmux identity from `node.created` event
/// data. Returns `Some` only when both `tmux_session` and `tmux_window_id` are
/// present and non-empty — the minimum needed to match a window. `tmux_socket`
/// is optional (a default-socket spawn may emit null); an empty socket is
/// normalized to `None` so the watchdog never invokes `tmux -S ""`. Legacy
/// events from a create.sh that predates the qualified fields (or that emit a
/// partial/empty identity) yield `None`, so the node falls back to bare-name
/// matching on `tmux_window`.
fn tmux_identity_from_data(d: &Value) -> Option<TmuxIdentity> {
    let nonempty = |key| {
        d.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let session = nonempty("tmux_session")?;
    let window_id = nonempty("tmux_window_id")?;
    Some(TmuxIdentity {
        socket: nonempty("tmux_socket"),
        session,
        window_id,
    })
}

fn reduce_node_created(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    // The envelope `node_id` is already a validated `NodeId` (parsed on read),
    // so take it directly — no re-parse needed.
    let node_id = require_envelope_node_id(&events_path, ev)?;
    // Idempotent on replay: skip if the node already exists.
    if read_node_opt(paths, &node_id)?.is_some() {
        return Ok(vec![]);
    }
    let d = &ev.data;
    let kind =
        data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=node.created missing/invalid `kind`",
                ev.seq
            ),
        })?;
    let n = Node {
        schema_version: STATE_SCHEMA_VERSION,
        node_id,
        // `run_id == paths.run_id` was verified at `reduce_event_to_ops` entry.
        run_id: paths.run_id.clone(),
        parent_node_id: opt_node_id(&events_path, ev, d, "parent_node_id")?,
        kind,
        status: Status::Pending,
        task: d.get("task").and_then(Value::as_str).map(str::to_string),
        worktree_path: d
            .get("worktree_path")
            .and_then(Value::as_str)
            .map(str::to_string),
        branch: d.get("branch").and_then(Value::as_str).map(str::to_string),
        tmux_window: d
            .get("tmux_window")
            .and_then(Value::as_str)
            .map(str::to_string),
        tmux_identity: tmux_identity_from_data(d),
        agent_pid: optional_i32(d, "agent_pid", &events_path, ev)?,
        agent_pid_start_time: optional_ts(d, "agent_pid_start_time", &events_path, ev)?,
        supervisor_pid: optional_i32(d, "supervisor_pid", &events_path, ev)?,
        children: Vec::new(),
        started_at: Some(ev.ts),
        updated_at: ev.ts,
        last_report: None,
        last_processed_report_seq_by_child: serde_json::Map::default(),
    };
    let mut ops = vec![ProjectionOp::Node(n)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `node_count` is derived from the projection directories in
        // `advance_applied_seq`, never incremented here — see the module note
        // and issue `manifest-counter-desync`. This op only refreshes the run's
        // last-activity timestamp.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_node_status(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(vec![]),
    };
    let new_status = require_status(ev, events_path)?;
    // Terminal-state guard: a settled node never transitions again. See
    // run-cli-read/handoff.md D5.
    if n.status.is_terminal() {
        trace_terminal_noop(ev, n.status, new_status);
        return Ok(vec![]);
    }
    if n.status == new_status {
        return Ok(vec![]);
    }
    n.status = new_status;
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

fn reduce_node_report(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(vec![]),
    };
    // Terminal-state guard *before* payload validation: a node that already
    // reached a terminal state is settled, so a late-arriving report (e.g. an
    // agent success racing a `run cancel`) is a dead event — it must not
    // resurrect the node, and must not even decorate the projection, so
    // `last_report` is left untouched. Guarding first also keeps replay
    // robust: a malformed dead report against a settled node is a clean
    // no-op rather than a `CorruptEventLog` that would brick rebuild of a
    // log `append_and_apply_event` already committed. See run-cli-read/handoff.md
    // D5. (3/4 of /llm-review preferred guard-before-validate over the
    // reverse the issue spec sketched; the required CorruptEventLog cases
    // all target live nodes, so validation still runs for them.)
    if n.status.is_terminal() {
        tracing::debug!(
            target: "octl_core::reducer",
            seq = ev.seq, kind = %ev.kind, node_id = %node_id, current = ?n.status,
            "no-op: node.report against terminal node"
        );
        return Ok(vec![]);
    }
    // Live node: validate the report's terminal outcome. A `node.report`
    // must express exactly one terminal outcome — success/failure XOR
    // cancellation. Anything else (a bare `{}` with neither, or the
    // contradiction `success: true` + `cancelled: true`) is a corrupt event:
    // the reducer is the canonical gate, so reject it rather than silently
    // leaving the node in a dangling state. See design.md §7.7 and
    // node-cli-read/handoff.md D4.
    let new_status = report_terminal_status(&events_path, ev)?;
    n.last_report = Some(ev.data.clone());
    n.status = new_status;
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

/// Emit an observability trace for a status event dropped by the terminal
/// guard. Re-applying the *same* terminal status is routine idempotent replay
/// (`debug`); an event carrying a *different* status is a real conflict that
/// should not occur on a well-formed log (`warn`) — e.g. a `done` node being
/// told to go `cancelled`. The guard no-ops either way; the level is the only
/// difference, so a genuine corruption signal is visible without flooding
/// logs on every replay.
fn trace_terminal_noop(ev: &Event, current: Status, incoming: Status) {
    if current == incoming {
        tracing::debug!(
            target: "octl_core::reducer",
            seq = ev.seq, kind = %ev.kind, status = ?current,
            "no-op: status re-applied to terminal target"
        );
    } else {
        tracing::warn!(
            target: "octl_core::reducer",
            seq = ev.seq, kind = %ev.kind, current = ?current, incoming = ?incoming,
            "no-op: ignored conflicting transition from terminal target"
        );
    }
}

/// Derive the terminal status a `node.report` event asserts, enforcing the
/// success-XOR-cancelled invariant with strict boolean typing.
///
/// `cancelled: true` (with `success: false` or absent) → [`Status::Cancelled`].
/// Otherwise `success` must be present: `true` → [`Status::Done`], `false` →
/// [`Status::Failed`]. Neither field (bare `{}`), the contradiction
/// `success: true` + `cancelled: true`, or a non-boolean `success` /
/// `cancelled` is a [`Error::CorruptEventLog`].
fn report_terminal_status(events_path: &Path, ev: &Event) -> Result<Status> {
    let corrupt = |reason: String| Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason,
    };
    let cancelled = optional_bool(events_path, ev, &ev.data, "cancelled")?.unwrap_or(false);
    let success = optional_bool(events_path, ev, &ev.data, "success")?;
    if cancelled {
        if success == Some(true) {
            return Err(corrupt(format!(
                "event seq={} kind=node.report has contradictory `success: true` with `cancelled: true`",
                ev.seq
            )));
        }
        Ok(Status::Cancelled)
    } else {
        match success {
            Some(true) => Ok(Status::Done),
            Some(false) => Ok(Status::Failed),
            None => Err(corrupt(format!(
                "event seq={} kind=node.report must set boolean `success` or `cancelled: true`",
                ev.seq
            ))),
        }
    }
}

fn reduce_discussion_opened(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let d = &ev.data;
    let discussion_id = DiscussionId::parse_str(want_str(&events_path, ev, d, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_discussion_opt(paths, &discussion_id)?.is_some() {
        return Ok(vec![]);
    }
    let node_id = want_node_id_with_fallback(&events_path, ev, d, "node_id")?;
    let options = d
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let disc = Discussion {
        schema_version: STATE_SCHEMA_VERSION,
        discussion_id,
        run_id: paths.run_id.clone(),
        node_id,
        opened_at: ev.ts,
        severity: d
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or("discuss")
            .to_string(),
        topic: want_str(&events_path, ev, d, "topic")?.to_string(),
        context: d.get("context").and_then(Value::as_str).map(str::to_string),
        options,
        status: DiscussionStatus::Open,
        resolution: None,
        note: None,
        resolved_at: None,
    };
    let mut ops = vec![ProjectionOp::Discussion(disc)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `open_discussions` is derived in `advance_applied_seq`, not bumped
        // here — see the module note. Only the timestamp is refreshed.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_discussion_resolved(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let id = DiscussionId::parse_str(want_str(&events_path, ev, &ev.data, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut disc = match read_discussion_opt(paths, &id)? {
        Some(d) => d,
        None => return Ok(vec![]),
    };
    if matches!(disc.status, DiscussionStatus::Resolved) {
        return Ok(vec![]);
    }
    disc.status = DiscussionStatus::Resolved;
    // `discussion.resolved` must carry a string `resolution` — without
    // one, the projection would advance to `Resolved` with `resolution:
    // null`, which is a corrupt domain state. Reject at the reducer
    // boundary so any writer (CLI, future supervisor, manual `event
    // create`) is held to the same contract.
    disc.resolution = Some(want_str(&events_path, ev, &ev.data, "resolution")?.to_string());
    disc.note = optional_str(&events_path, ev, &ev.data, "note")?;
    disc.resolved_at = Some(ev.ts);
    let mut ops = vec![ProjectionOp::Discussion(disc)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `open_discussions` is derived in `advance_applied_seq`, not
        // decremented here — see the module note. The old `saturating_sub`
        // could strand a too-high count if this resolve's manifest write was
        // lost to a crash and the replay then short-circuited on the
        // already-`Resolved` discussion. Only the timestamp is refreshed.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_spinoff_proposed(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let d = &ev.data;
    let proposal_id = ProposalId::parse_str(want_str(&events_path, ev, d, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_spinoff_opt(paths, &proposal_id)?.is_some() {
        return Ok(vec![]);
    }
    let proposed_kind =
        data_kind(d.get("proposed_kind").unwrap_or(&Value::Null)).ok_or_else(|| {
            Error::CorruptEventLog {
                path: events_path.clone(),
                reason: format!(
                    "event seq={} kind=spinoff.proposed missing/invalid `proposed_kind`",
                    ev.seq
                ),
            }
        })?;
    let node_id = want_node_id_with_fallback(&events_path, ev, d, "node_id")?;
    let s = SpinoffProposal {
        schema_version: STATE_SCHEMA_VERSION,
        proposal_id,
        run_id: paths.run_id.clone(),
        node_id,
        proposed_at: ev.ts,
        proposed_title: want_str(&events_path, ev, d, "proposed_title")?.to_string(),
        proposed_kind,
        rationale: d
            .get("rationale")
            .and_then(Value::as_str)
            .map(str::to_string),
        status: SpinoffStatus::Proposed,
        accepted_as_issue_slug: None,
        rejected_reason: None,
        resolved_at: None,
    };
    let mut ops = vec![ProjectionOp::Spinoff(s)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `pending_spinoffs` is derived in `advance_applied_seq`, not bumped
        // here — see the module note. Only the timestamp is refreshed.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_spinoff_approved(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let id = ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut s = match read_spinoff_opt(paths, &id)? {
        Some(s) => s,
        None => return Ok(vec![]),
    };
    if matches!(s.status, SpinoffStatus::Approved | SpinoffStatus::Rejected) {
        return Ok(vec![]);
    }
    s.status = SpinoffStatus::Approved;
    s.accepted_as_issue_slug = ev
        .data
        .get("issue_slug")
        .and_then(Value::as_str)
        .map(str::to_string);
    s.resolved_at = Some(ev.ts);
    let mut ops = vec![ProjectionOp::Spinoff(s)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `pending_spinoffs` is derived in `advance_applied_seq`, not
        // decremented here — see the module note. The old `saturating_sub`
        // could strand a too-high count if this resolution's manifest write was
        // lost to a crash and the replay then short-circuited on the
        // already-settled proposal. Only the timestamp is refreshed.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_spinoff_rejected(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let id = ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut s = match read_spinoff_opt(paths, &id)? {
        Some(s) => s,
        None => return Ok(vec![]),
    };
    if matches!(s.status, SpinoffStatus::Approved | SpinoffStatus::Rejected) {
        return Ok(vec![]);
    }
    s.status = SpinoffStatus::Rejected;
    s.rejected_reason = ev
        .data
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    s.resolved_at = Some(ev.ts);
    let mut ops = vec![ProjectionOp::Spinoff(s)];
    if let Some(mut m) = read_manifest_opt(paths)? {
        // `pending_spinoffs` is derived in `advance_applied_seq`, not
        // decremented here — see the module note. The old `saturating_sub`
        // could strand a too-high count if this resolution's manifest write was
        // lost to a crash and the replay then short-circuited on the
        // already-settled proposal. Only the timestamp is refreshed.
        m.updated_at = ev.ts;
        ops.push(ProjectionOp::Manifest(m));
    }
    Ok(ops)
}

fn reduce_child_spawned(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    // `child.spawned` is written to the PARENT run's events; the parent
    // spawning node is `ev.node_id`, the child run/node lives in `data`.
    let events_path = paths.events();
    let parent_node_id = ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=child.spawned missing parent `node_id`",
            ev.seq
        ),
    })?;
    let child_run_id = RunId::parse_str(want_str(&events_path, ev, &ev.data, "child_run_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let child_node_id = NodeId::parse_str(
        ev.data
            .get("child_node_id")
            .and_then(Value::as_str)
            .unwrap_or("n-0001"),
    )
    .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut n = match read_node_opt(paths, &parent_node_id)? {
        Some(n) => n,
        None => return Ok(vec![]),
    };
    let new_ref = ChildRef {
        run_id: child_run_id,
        node_id: child_node_id,
    };
    if n.children.iter().any(|c| c == &new_ref) {
        // Already recorded — pure no-op so replayed events don't churn
        // `updated_at` or the projection file.
        return Ok(vec![]);
    }
    n.children.push(new_ref);
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

/// `supervisor.attached` records the supervisor PID watching the envelope
/// node onto `Node.supervisor_pid`. Event-sourced replacement for the
/// supervisor's former direct `write_node` (issue
/// `supervisor-state-not-event-sourced`), so a from-scratch projection
/// rebuild reproduces the field.
///
/// Latest-wins: a later attach (a supervisor restart binds a fresh PID)
/// overrides the recorded value. Re-applying an event that carries the
/// already-recorded PID is a pure no-op, so replay never churns the
/// projection file's `updated_at`.
fn reduce_supervisor_attached(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let raw = ev
        .data
        .get("pid")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=supervisor.attached missing/invalid `pid`",
                ev.seq
            ),
        })?;
    let pid = i32::try_from(raw).map_err(|_| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=supervisor.attached `pid` out of i32 range: {raw}",
            ev.seq
        ),
    })?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(vec![]),
    };
    if n.supervisor_pid == Some(pid) {
        return Ok(vec![]);
    }
    n.supervisor_pid = Some(pid);
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

/// `supervisor.cursor_advanced` mirrors the supervisor's per-child report
/// cursor onto the envelope (parent) node's `last_processed_report_seq_by_child`
/// map. Event-sourced replacement for the supervisor's former direct
/// `write_node` of that map (issue `supervisor-state-not-event-sourced`).
///
/// The cursor is monotonic: a `report_seq` at or below the recorded
/// high-water mark for this child is a no-op, so replaying the same event —
/// or an older out-of-order one — never moves the cursor backward or churns
/// the projection. This is the §7.3 idempotency guarantee at the reducer
/// boundary.
fn reduce_supervisor_cursor_advanced(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let child_run_id = want_str(&events_path, ev, &ev.data, "child_run_id")?;
    // Validate the child id even though it only becomes a map key — a forged
    // event must not smuggle a path-shaped or malformed run id into the
    // projection.
    RunId::parse_str(child_run_id).map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let report_seq = ev
        .data
        .get("report_seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=supervisor.cursor_advanced missing/invalid `report_seq`",
                ev.seq
            ),
        })?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(vec![]),
    };
    if let Some(prev) = n
        .last_processed_report_seq_by_child
        .get(child_run_id)
        .and_then(Value::as_u64)
    {
        if report_seq <= prev {
            return Ok(vec![]);
        }
    }
    n.last_processed_report_seq_by_child
        .insert(child_run_id.to_string(), Value::from(report_seq));
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Event;
    use chrono::Utc;
    use tempfile::TempDir;

    fn event(run_id: &str) -> Event {
        Event {
            ts: Utc::now(),
            seq: 1,
            kind: "run.status".into(),
            run_id: RunId::parse_str(run_id).unwrap(),
            node_id: None,
            idempotency_key: None,
            data: serde_json::json!({ "status": "running" }),
        }
    }

    #[test]
    fn orchestrator_decision_and_discuss_critical_reduce_to_noop() {
        // The /orchestrate audit kinds are append-only: the reducer must plan
        // ZERO projection ops for them regardless of payload, so the event log
        // is their sole home and no projection is created or mutated.
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let rid = RunId::parse_str(run_id).unwrap();
        let dir = crate::run_dir(tmp.path(), &rid);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();

        // Bootstrap a manifest so we can prove the audit events leave it
        // byte-for-byte untouched (no counter churn, no status drift).
        let mut created = event(run_id);
        created.kind = "run.created".into();
        created.data = serde_json::json!({
            "kind": "spinoff", "lifecycle": "autonomous", "title": "t"
        });
        apply_event(&paths, &created).expect("run.created applies");
        let manifest_before = std::fs::read(paths.manifest()).unwrap();

        for (seq, kind) in [(10u64, "orchestrator.decision"), (11, "discuss.critical")] {
            let mut ev = event(run_id);
            ev.seq = seq;
            ev.kind = kind.into();
            // A non-trivial payload to prove the reducer ignores it wholesale.
            ev.data = serde_json::json!({ "summary": "x", "arbitrary": [1, 2, 3] });
            let ops = reduce_event_to_ops(&paths, &ev).expect("audit kind reduces cleanly");
            assert!(ops.is_empty(), "{kind} must plan no projection ops");
            // apply_event is the plan+commit path; it must also be a clean no-op.
            apply_event(&paths, &ev).expect("audit kind applies as no-op");
        }

        // The manifest is unchanged and no stray projection dirs appeared.
        assert_eq!(
            std::fs::read(paths.manifest()).unwrap(),
            manifest_before,
            "audit events must not mutate the manifest"
        );
        assert!(!paths.nodes_dir().exists(), "no node projection created");
    }

    #[test]
    fn apply_event_rejects_event_from_a_different_run() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let rid = RunId::parse_str(run_id).unwrap();
        let dir = crate::run_dir(tmp.path(), &rid);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();

        // An event whose envelope names a different run must not be folded.
        let foreign = event("02jxsnap000000000000000000");
        let err = apply_event(&paths, &foreign).expect_err("cross-run event must be rejected");
        assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");

        // The matching run_id is accepted (no projection exists yet, so
        // `run.status` is a clean no-op rather than an error).
        let mine = event(run_id);
        apply_event(&paths, &mine).expect("matching run_id must be accepted");
    }

    #[test]
    fn tmux_identity_from_data_reads_qualified_fields() {
        let d = serde_json::json!({
            "tmux_socket": "/private/tmp/tmux-501/default",
            "tmux_session": "octl",
            "tmux_window_id": "@42",
        });
        let id = tmux_identity_from_data(&d).expect("qualified identity");
        assert_eq!(id.socket.as_deref(), Some("/private/tmp/tmux-501/default"));
        assert_eq!(id.session, "octl");
        assert_eq!(id.window_id, "@42");

        // Null socket is tolerated — session + window_id are the minimum.
        let d2 = serde_json::json!({
            "tmux_socket": null,
            "tmux_session": "octl",
            "tmux_window_id": "@7",
        });
        let id2 = tmux_identity_from_data(&d2).expect("identity without socket");
        assert_eq!(id2.socket, None);
        assert_eq!(id2.window_id, "@7");
    }

    #[test]
    fn tmux_identity_from_data_back_compat_is_none() {
        // Legacy create.sh: no qualified fields at all.
        let legacy = serde_json::json!({ "tmux_window": "🚀 wt/x" });
        assert!(tmux_identity_from_data(&legacy).is_none());
        // Partial (window_id without session) is also insufficient → None.
        let partial = serde_json::json!({ "tmux_window_id": "@42" });
        assert!(tmux_identity_from_data(&partial).is_none());
    }

    /// End-to-end: a `node.created` event carrying the qualified fields folds
    /// them into `Node.tmux_identity`; one without them leaves it `None`.
    #[test]
    fn node_created_populates_tmux_identity() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let rid = RunId::parse_str(run_id).unwrap();
        let dir = crate::run_dir(tmp.path(), &rid);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();

        let mut ev = event(run_id);
        ev.seq = 2;
        ev.kind = "node.created".into();
        ev.node_id = Some(NodeId::parse_str("n-0001").unwrap());
        ev.data = serde_json::json!({
            "kind": "spinoff",
            "tmux_window": "🚀 wt/x",
            "tmux_socket": "/private/tmp/tmux-501/default",
            "tmux_session": "octl",
            "tmux_window_id": "@42",
        });
        apply_event(&paths, &ev).expect("node.created applies");
        let n = read_node_opt(&paths, &NodeId::parse_str("n-0001").unwrap())
            .unwrap()
            .unwrap();
        let id = n.tmux_identity.expect("qualified identity recorded");
        assert_eq!(id.session, "octl");
        assert_eq!(id.window_id, "@42");
        assert_eq!(n.tmux_window.as_deref(), Some("🚀 wt/x"));

        // A second run with a legacy event leaves tmux_identity None.
        let run2 = "02jxsnap000000000000000000";
        let rid2 = RunId::parse_str(run2).unwrap();
        let dir2 = crate::run_dir(tmp.path(), &rid2);
        std::fs::create_dir_all(&dir2).unwrap();
        let paths2 = RunPaths::new(dir2, run2).unwrap();
        let mut ev2 = event(run2);
        ev2.seq = 2;
        ev2.kind = "node.created".into();
        ev2.node_id = Some(NodeId::parse_str("n-0001").unwrap());
        ev2.data = serde_json::json!({ "kind": "spinoff", "tmux_window": "🚀 wt/y" });
        apply_event(&paths2, &ev2).expect("legacy node.created applies");
        let n2 = read_node_opt(&paths2, &NodeId::parse_str("n-0001").unwrap())
            .unwrap()
            .unwrap();
        assert!(n2.tmux_identity.is_none());
        assert_eq!(n2.tmux_window.as_deref(), Some("🚀 wt/y"));
    }

    /// Bootstrap a run with a single `n-0001` node via the event-sourced path,
    /// returning its paths. Shared by the supervisor-state replay tests below.
    fn seed_run_with_node(tmp: &TempDir, run_id: &str) -> RunPaths {
        let rid = RunId::parse_str(run_id).unwrap();
        let dir = crate::run_dir(tmp.path(), &rid);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();

        let mut created = event(run_id);
        created.kind = "run.created".into();
        created.data = serde_json::json!({
            "kind": "spinoff", "lifecycle": "autonomous", "title": "t"
        });
        apply_event(&paths, &created).expect("run.created applies");

        let mut node = event(run_id);
        node.seq = 2;
        node.kind = "node.created".into();
        node.node_id = Some(NodeId::parse_str("n-0001").unwrap());
        node.data = serde_json::json!({ "kind": "spinoff" });
        apply_event(&paths, &node).expect("node.created applies");
        paths
    }

    fn read_n0001(paths: &RunPaths) -> Node {
        read_node_opt(paths, &NodeId::parse_str("n-0001").unwrap())
            .unwrap()
            .unwrap()
    }

    /// Replaying `supervisor.attached` from scratch reproduces
    /// `Node.supervisor_pid` — the field is now event-sourced, not a
    /// projection-only write (issue `supervisor-state-not-event-sourced`).
    #[test]
    fn supervisor_attached_sets_supervisor_pid() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = seed_run_with_node(&tmp, run_id);
        assert_eq!(read_n0001(&paths).supervisor_pid, None);

        let mut ev = event(run_id);
        ev.seq = 3;
        ev.kind = "supervisor.attached".into();
        ev.node_id = Some(NodeId::parse_str("n-0001").unwrap());
        ev.data = serde_json::json!({ "pid": 47820 });
        apply_event(&paths, &ev).expect("supervisor.attached applies");
        assert_eq!(read_n0001(&paths).supervisor_pid, Some(47820));
    }

    /// A second attach with a different pid overrides (latest-wins); a replay
    /// of the *same* pid is a pure no-op that does not churn `updated_at`.
    #[test]
    fn supervisor_attached_latest_wins_and_idempotent_on_replay() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = seed_run_with_node(&tmp, run_id);

        let mut ev = event(run_id);
        ev.kind = "supervisor.attached".into();
        ev.node_id = Some(NodeId::parse_str("n-0001").unwrap());

        ev.seq = 3;
        ev.data = serde_json::json!({ "pid": 100 });
        apply_event(&paths, &ev).expect("first attach applies");
        assert_eq!(read_n0001(&paths).supervisor_pid, Some(100));

        // A restart binds a fresh pid: latest-wins.
        ev.seq = 4;
        ev.data = serde_json::json!({ "pid": 200 });
        apply_event(&paths, &ev).expect("second attach applies");
        let after_second = read_n0001(&paths);
        assert_eq!(after_second.supervisor_pid, Some(200));

        // Replaying the latest event again is a no-op: the planned ops are
        // empty and the projection bytes (including `updated_at`) are unchanged.
        let ops = reduce_event_to_ops(&paths, &ev).expect("replay reduces cleanly");
        assert!(ops.is_empty(), "re-applying same pid must plan no ops");
        apply_event(&paths, &ev).expect("replay applies as no-op");
        assert_eq!(read_n0001(&paths).updated_at, after_second.updated_at);
    }

    /// Replaying `supervisor.cursor_advanced` from scratch reproduces
    /// `Node.last_processed_report_seq_by_child`.
    #[test]
    fn supervisor_cursor_advanced_sets_report_cursor() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = seed_run_with_node(&tmp, run_id);
        let child = "02jxsnap000000000000000000";

        let mut ev = event(run_id);
        ev.seq = 3;
        ev.kind = "supervisor.cursor_advanced".into();
        ev.node_id = Some(NodeId::parse_str("n-0001").unwrap());
        ev.data = serde_json::json!({ "child_run_id": child, "report_seq": 7 });
        apply_event(&paths, &ev).expect("cursor_advanced applies");

        let n = read_n0001(&paths);
        assert_eq!(
            n.last_processed_report_seq_by_child.get(child),
            Some(&Value::from(7u64))
        );
    }

    /// The cursor is monotonic and idempotent: re-applying the same
    /// `(child_run_id, report_seq)` is a no-op, an older seq never moves the
    /// cursor backward, and a higher seq advances it. A second distinct child
    /// gets its own independent entry.
    #[test]
    fn supervisor_cursor_advanced_is_monotonic_and_idempotent() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = seed_run_with_node(&tmp, run_id);
        let child_a = "02jxsnap000000000000000000";
        let child_b = "03jxsnap000000000000000000";

        let mut ev = event(run_id);
        ev.kind = "supervisor.cursor_advanced".into();
        ev.node_id = Some(NodeId::parse_str("n-0001").unwrap());

        ev.seq = 3;
        ev.data = serde_json::json!({ "child_run_id": child_a, "report_seq": 5 });
        apply_event(&paths, &ev).expect("seq 5 applies");

        // Replay the exact same event — no-op, plans zero ops.
        let ops = reduce_event_to_ops(&paths, &ev).expect("replay reduces cleanly");
        assert!(ops.is_empty(), "re-applying same cursor must plan no ops");

        // An older seq must not move the cursor backward.
        ev.seq = 4;
        ev.data = serde_json::json!({ "child_run_id": child_a, "report_seq": 3 });
        let ops = reduce_event_to_ops(&paths, &ev).expect("older seq reduces cleanly");
        assert!(ops.is_empty(), "older seq must plan no ops");
        apply_event(&paths, &ev).expect("older seq applies as no-op");
        assert_eq!(
            read_n0001(&paths)
                .last_processed_report_seq_by_child
                .get(child_a),
            Some(&Value::from(5u64))
        );

        // A higher seq advances; an independent child gets its own entry.
        ev.seq = 5;
        ev.data = serde_json::json!({ "child_run_id": child_a, "report_seq": 9 });
        apply_event(&paths, &ev).expect("higher seq applies");
        ev.seq = 6;
        ev.data = serde_json::json!({ "child_run_id": child_b, "report_seq": 1 });
        apply_event(&paths, &ev).expect("second child applies");

        let n = read_n0001(&paths);
        assert_eq!(
            n.last_processed_report_seq_by_child.get(child_a),
            Some(&Value::from(9u64))
        );
        assert_eq!(
            n.last_processed_report_seq_by_child.get(child_b),
            Some(&Value::from(1u64))
        );
    }

    /// Both new kinds reject a malformed payload at the reducer boundary so a
    /// forged event can never write a corrupt projection.
    #[test]
    fn supervisor_state_events_reject_malformed_payloads() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = seed_run_with_node(&tmp, run_id);
        let nid = Some(NodeId::parse_str("n-0001").unwrap());

        // Missing pid.
        let mut ev = event(run_id);
        ev.seq = 3;
        ev.kind = "supervisor.attached".into();
        ev.node_id = nid.clone();
        ev.data = serde_json::json!({});
        assert!(matches!(
            reduce_event_to_ops(&paths, &ev),
            Err(Error::CorruptEventLog { .. })
        ));

        // Missing envelope node_id.
        ev.node_id = None;
        ev.data = serde_json::json!({ "pid": 1 });
        assert!(matches!(
            reduce_event_to_ops(&paths, &ev),
            Err(Error::CorruptEventLog { .. })
        ));

        // cursor_advanced: malformed child_run_id.
        let mut ev2 = event(run_id);
        ev2.seq = 4;
        ev2.kind = "supervisor.cursor_advanced".into();
        ev2.node_id = nid.clone();
        ev2.data = serde_json::json!({ "child_run_id": "../etc", "report_seq": 1 });
        assert!(matches!(
            reduce_event_to_ops(&paths, &ev2),
            Err(Error::CorruptEventLog { .. })
        ));

        // cursor_advanced: missing report_seq.
        ev2.data = serde_json::json!({ "child_run_id": "02jxsnap000000000000000000" });
        assert!(matches!(
            reduce_event_to_ops(&paths, &ev2),
            Err(Error::CorruptEventLog { .. })
        ));
    }

    /// Snapshot every projection file under `paths` to a `path → inode` map.
    ///
    /// An atomic projection write is temp-file + rename, so a rewritten file
    /// always lands a *fresh inode* — even when its bytes are byte-for-byte
    /// identical (e.g. a manifest op that refreshes `updated_at` to the same
    /// timestamp). Comparing inodes therefore detects every write the reducer
    /// makes, with no false negatives a content diff would suffer. `events.jsonl`
    /// and `.lock` are excluded: `apply_event` never touches them.
    #[cfg(unix)]
    fn projection_inodes(paths: &RunPaths) -> std::collections::BTreeMap<PathBuf, u64> {
        use std::os::unix::fs::MetadataExt;
        let mut consider = vec![paths.manifest()];
        for dir in [
            paths.nodes_dir(),
            paths.discussions_dir(),
            paths.spinoffs_dir(),
        ] {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("json") {
                        consider.push(p);
                    }
                }
            }
        }
        let mut map = std::collections::BTreeMap::new();
        for p in consider {
            if let Ok(md) = std::fs::symlink_metadata(&p) {
                if md.file_type().is_file() {
                    map.insert(p, md.ino());
                }
            }
        }
        map
    }

    /// The exhaustive parity guarantee `projected-paths-into-reducer` requires:
    /// for an event applied against a given state, the paths
    /// [`plan_projections`] reports MUST equal the files [`apply_event`]
    /// actually writes. Plan first (against pre-apply state), apply, then diff
    /// the projection inodes — a file is "written" iff it is newly present or
    /// its inode changed. `expect_writes` guards the test itself: when set, the
    /// touched set must be non-empty, so a kind that silently stopped writing
    /// can't pass by matching an empty plan against an empty diff.
    #[cfg(unix)]
    fn assert_plan_matches_apply(paths: &RunPaths, ev: &Event, expect_writes: bool) {
        use std::collections::BTreeSet;
        let before = projection_inodes(paths);
        let planned: BTreeSet<PathBuf> = plan_projections(paths, ev)
            .unwrap_or_else(|e| panic!("plan_projections({}) errored: {e:?}", ev.kind))
            .into_iter()
            .collect();
        apply_event(paths, ev)
            .unwrap_or_else(|e| panic!("apply_event({}) errored: {e:?}", ev.kind));
        let after = projection_inodes(paths);
        let touched: BTreeSet<PathBuf> = after
            .iter()
            .filter(|(p, ino)| before.get(*p) != Some(*ino))
            .map(|(p, _)| p.clone())
            .collect();
        assert_eq!(
            planned, touched,
            "kind={}: plan_projections must name exactly the files apply_event writes",
            ev.kind
        );
        if expect_writes {
            assert!(
                !touched.is_empty(),
                "kind={}: expected this event to write at least one projection",
                ev.kind
            );
        }
    }

    /// Drive every event kind through a dependency-ordered lifecycle on real
    /// runs, asserting plan/apply parity at each step. Covers the writing kinds
    /// (run/node/discussion/spinoff/supervisor/child) in states where they
    /// project, plus the no-op kinds (audit records, `supervisor.exited`,
    /// terminal-guarded transitions) where both the plan and the apply touch
    /// nothing.
    #[cfg(unix)]
    #[test]
    fn plan_projections_matches_apply_for_every_kind() {
        let tmp = TempDir::new().unwrap();
        let run_id = "01jxsnap000000000000000000";
        let rid = RunId::parse_str(run_id).unwrap();
        let dir = crate::run_dir(tmp.path(), &rid);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();
        let nid = || Some(NodeId::parse_str("n-0001").unwrap());
        let disc_id = "d-pqrstuvwxy";
        let prop_id = "s-spinaaaaaa";
        let child = "02jxsnap000000000000000000";

        // Helper to build a fresh envelope at a monotonic seq.
        let mut next_seq = 0u64;
        let mut at = |kind: &str, node_id, data| {
            next_seq += 1;
            Event {
                ts: Utc::now(),
                seq: next_seq,
                kind: kind.into(),
                run_id: rid.clone(),
                node_id,
                idempotency_key: None,
                data,
            }
        };

        // run.created → manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "run.created",
                None,
                serde_json::json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
            ),
            true,
        );
        // run.status (pending → running) → manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "run.status",
                None,
                serde_json::json!({ "status": "running" }),
            ),
            true,
        );
        // node.created → nodes/n-0001.json + manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "node.created",
                nid(),
                serde_json::json!({ "kind": "spinoff" }),
            ),
            true,
        );
        // node.status (pending → running) → nodes/n-0001.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "node.status",
                nid(),
                serde_json::json!({ "status": "running" }),
            ),
            true,
        );
        // discussion.opened → discussions/<id>.json + manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "discussion.opened",
                None,
                serde_json::json!({ "discussion_id": disc_id, "node_id": "n-0001", "topic": "t" }),
            ),
            true,
        );
        // discussion.resolved → discussions/<id>.json + manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "discussion.resolved",
                None,
                serde_json::json!({ "discussion_id": disc_id, "resolution": "keep" }),
            ),
            true,
        );
        // spinoff.proposed → spinoffs/<id>.json + manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "spinoff.proposed",
                None,
                serde_json::json!({
                    "proposal_id": prop_id, "node_id": "n-0001",
                    "proposed_title": "p", "proposed_kind": "spinoff"
                }),
            ),
            true,
        );
        // spinoff.approved → spinoffs/<id>.json + manifest.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "spinoff.approved",
                None,
                serde_json::json!({ "proposal_id": prop_id, "issue_slug": "x" }),
            ),
            true,
        );
        // supervisor.attached → nodes/n-0001.json (still non-terminal)
        assert_plan_matches_apply(
            &paths,
            &at(
                "supervisor.attached",
                nid(),
                serde_json::json!({ "pid": 4242 }),
            ),
            true,
        );
        // supervisor.cursor_advanced → nodes/n-0001.json
        assert_plan_matches_apply(
            &paths,
            &at(
                "supervisor.cursor_advanced",
                nid(),
                serde_json::json!({ "child_run_id": child, "report_seq": 3 }),
            ),
            true,
        );
        // child.spawned → nodes/n-0001.json (parent node)
        assert_plan_matches_apply(
            &paths,
            &at(
                "child.spawned",
                nid(),
                serde_json::json!({ "child_run_id": child, "child_node_id": "n-0001" }),
            ),
            true,
        );
        // node.report success → nodes/n-0001.json (now terminal)
        assert_plan_matches_apply(
            &paths,
            &at("node.report", nid(), serde_json::json!({ "success": true })),
            true,
        );
        // Terminal-guarded no-ops: a settled node swallows further transitions,
        // so both the plan and the apply touch nothing.
        assert_plan_matches_apply(
            &paths,
            &at(
                "node.status",
                nid(),
                serde_json::json!({ "status": "failed" }),
            ),
            false,
        );
        // No-op audit / lifecycle kinds: zero projections by design.
        for kind in [
            "supervisor.exited",
            "orchestrator.decision",
            "discuss.critical",
            "cleanup.window_missing",
        ] {
            assert_plan_matches_apply(&paths, &at(kind, None, serde_json::json!({})), false);
        }

        // spinoff.rejected needs its own un-settled proposal — exercise it on a
        // second proposal id so the approve above doesn't shadow it.
        let prop2 = "s-spinbbbbbb";
        assert_plan_matches_apply(
            &paths,
            &at(
                "spinoff.proposed",
                None,
                serde_json::json!({
                    "proposal_id": prop2, "node_id": "n-0001",
                    "proposed_title": "p2", "proposed_kind": "spinoff"
                }),
            ),
            true,
        );
        assert_plan_matches_apply(
            &paths,
            &at(
                "spinoff.rejected",
                None,
                serde_json::json!({ "proposal_id": prop2, "reason": "no" }),
            ),
            true,
        );
    }
}
