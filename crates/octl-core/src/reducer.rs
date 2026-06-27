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
//! projections must not double-count manifest counters.

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
    STATE_SCHEMA_VERSION,
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

/// Apply one event to projections. No-op for unknown `kind`.
///
/// Caller must hold the run's [`crate::lock::RunLock`].
///
/// `pub(crate)`: applying an event in isolation (without the matching
/// `events.jsonl` append) is an internal building block of
/// [`crate::events::append_and_apply_unlocked`] and a future
/// `rebuild_projections_from_events`. External callers mutate state through
/// [`crate::events::append_and_apply_event`] so the log and projections can
/// never diverge.
pub(crate) fn apply_event(paths: &RunPaths, ev: &Event) -> Result<()> {
    // An event whose envelope `run_id` doesn't match the run we're folding it
    // into means the log was copied/misrouted — fold it and projections would
    // be silently cross-contaminated. Reject before any write.
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
        "run.created" => apply_run_created(paths, ev),
        "run.status" => apply_run_status(paths, ev),
        "node.created" => apply_node_created(paths, ev),
        "node.status" => apply_node_status(paths, ev),
        "node.report" => apply_node_report(paths, ev),
        "discussion.opened" => apply_discussion_opened(paths, ev),
        "discussion.resolved" => apply_discussion_resolved(paths, ev),
        "spinoff.proposed" => apply_spinoff_proposed(paths, ev),
        "spinoff.approved" => apply_spinoff_approved(paths, ev),
        "spinoff.rejected" => apply_spinoff_rejected(paths, ev),
        "child.spawned" => apply_child_spawned(paths, ev),
        "supervisor.exited" => Ok(()),
        _ => Ok(()),
    }
}

/// Validate an event WITHOUT writing anything, returning `Err` in exactly
/// the cases [`apply_event`] would (for the same projection state) and `Ok`
/// otherwise.
///
/// This is the transactional gate run *before* the durable append in
/// [`crate::events::append_and_apply_unlocked`]: a reducer-rejected event is
/// caught here and never reaches `events.jsonl`, so a later replay /
/// `rebuild_projections` can't trip over a poison line
/// (append-and-apply-transactional-validation).
///
/// It mirrors `apply_event` branch-for-branch, including the state-dependent
/// no-op guards (a settled node/run/discussion swallows a late or even
/// malformed event as a clean no-op, so validation must NOT reject it). The
/// read-only projection reads here see the same state `apply_event` will,
/// because the caller holds the run's [`crate::lock::RunLock`] across both.
///
/// `pub(crate)`: an internal building block of `append_and_apply_unlocked`.
/// Keep it in lockstep with `apply_event` — the
/// `validate_matches_apply_rejection` test asserts they agree.
pub(crate) fn validate_event(paths: &RunPaths, ev: &Event) -> Result<()> {
    // Same cross-run guard as `apply_event`'s entry.
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
    #[allow(clippy::match_same_arms)]
    match ev.kind.as_str() {
        "run.created" => validate_run_created(paths, ev),
        "run.status" => validate_run_status(paths, ev),
        "node.created" => validate_node_created(paths, ev),
        "node.status" => validate_node_status(paths, ev),
        "node.report" => validate_node_report(paths, ev),
        "discussion.opened" => validate_discussion_opened(paths, ev),
        "discussion.resolved" => validate_discussion_resolved(paths, ev),
        "spinoff.proposed" => validate_spinoff_proposed(paths, ev),
        "spinoff.approved" => validate_spinoff_approved(paths, ev),
        "spinoff.rejected" => validate_spinoff_rejected(paths, ev),
        "child.spawned" => validate_child_spawned(paths, ev),
        "supervisor.exited" => Ok(()),
        _ => Ok(()),
    }
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

fn validate_run_created(paths: &RunPaths, ev: &Event) -> Result<()> {
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
        return Ok(());
    }
    let events_path = paths.events();
    let d = &ev.data;
    data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: "run.created missing/invalid `kind`".into(),
    })?;
    serde_json::from_value::<Lifecycle>(d.get("lifecycle").cloned().unwrap_or(Value::Null))
        .map_err(|_| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: "run.created missing/invalid `lifecycle`".into(),
        })?;
    want_str(&events_path, ev, d, "title")?;
    opt_run_id(&events_path, ev, d, "parent_run_id")?;
    opt_node_id(&events_path, ev, d, "parent_node_id")?;
    Ok(())
}

fn validate_run_status(paths: &RunPaths, ev: &Event) -> Result<()> {
    // Missing manifest → `apply_run_status` no-ops without inspecting status.
    if read_manifest_opt(paths)?.is_none() {
        return Ok(());
    }
    // `require_status` runs whether or not the run is terminal (the terminal
    // guard only short-circuits *after* it), so validate it unconditionally.
    require_status(ev, paths.events())?;
    Ok(())
}

fn validate_node_created(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    if read_node_opt(paths, &node_id)?.is_some() {
        return Ok(());
    }
    let d = &ev.data;
    data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=node.created missing/invalid `kind`",
            ev.seq
        ),
    })?;
    opt_node_id(&events_path, ev, d, "parent_node_id")?;
    optional_i32(d, "agent_pid", &events_path, ev)?;
    optional_ts(d, "agent_pid_start_time", &events_path, ev)?;
    optional_i32(d, "supervisor_pid", &events_path, ev)?;
    Ok(())
}

fn validate_node_status(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    // Missing node → no-op (no status validation). A live OR terminal node
    // both run `require_status` (terminal guard short-circuits after it).
    if read_node_opt(paths, &node_id)?.is_none() {
        return Ok(());
    }
    require_status(ev, events_path)?;
    Ok(())
}

fn validate_node_report(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    // Terminal-state guard runs BEFORE payload validation in `apply_node_report`:
    // a late report against a settled node is a dead no-op, even if malformed.
    if n.status.is_terminal() {
        return Ok(());
    }
    report_terminal_status(&events_path, ev)?;
    Ok(())
}

fn validate_discussion_opened(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let d = &ev.data;
    let discussion_id = DiscussionId::parse_str(want_str(&events_path, ev, d, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_discussion_opt(paths, &discussion_id)?.is_some() {
        return Ok(());
    }
    want_node_id_with_fallback(&events_path, ev, d, "node_id")?;
    want_str(&events_path, ev, d, "topic")?;
    Ok(())
}

fn validate_discussion_resolved(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let id = DiscussionId::parse_str(want_str(&events_path, ev, &ev.data, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let disc = match read_discussion_opt(paths, &id)? {
        Some(d) => d,
        None => return Ok(()),
    };
    if matches!(disc.status, DiscussionStatus::Resolved) {
        return Ok(());
    }
    want_str(&events_path, ev, &ev.data, "resolution")?;
    optional_str(&events_path, ev, &ev.data, "note")?;
    Ok(())
}

fn validate_spinoff_proposed(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let d = &ev.data;
    let proposal_id = ProposalId::parse_str(want_str(&events_path, ev, d, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_spinoff_opt(paths, &proposal_id)?.is_some() {
        return Ok(());
    }
    data_kind(d.get("proposed_kind").unwrap_or(&Value::Null)).ok_or_else(|| {
        Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=spinoff.proposed missing/invalid `proposed_kind`",
                ev.seq
            ),
        }
    })?;
    want_node_id_with_fallback(&events_path, ev, d, "node_id")?;
    want_str(&events_path, ev, d, "proposed_title")?;
    Ok(())
}

fn validate_spinoff_approved(paths: &RunPaths, ev: &Event) -> Result<()> {
    // `apply_spinoff_approved` can only reject on the id parse; the
    // existence/terminal guards and optional `issue_slug` never error.
    let events_path = paths.events();
    ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    Ok(())
}

fn validate_spinoff_rejected(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    Ok(())
}

fn validate_child_spawned(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    // All three id checks precede the parent-node existence no-op in
    // `apply_child_spawned`, so they are unconditional.
    ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=child.spawned missing parent `node_id`",
            ev.seq
        ),
    })?;
    RunId::parse_str(want_str(&events_path, ev, &ev.data, "child_run_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    NodeId::parse_str(
        ev.data
            .get("child_node_id")
            .and_then(Value::as_str)
            .unwrap_or("n-0001"),
    )
    .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    Ok(())
}

fn apply_run_created(paths: &RunPaths, ev: &Event) -> Result<()> {
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
        return Ok(());
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
        // `run_id == paths.run_id` was verified at `apply_event` entry.
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
        node_count: 0,
        open_discussions: 0,
        pending_spinoffs: 0,
        parent_run_id: opt_run_id(&events_path, ev, d, "parent_run_id")?,
        parent_node_id: opt_node_id(&events_path, ev, d, "parent_node_id")?,
    };
    write_manifest(paths, &m)
}

fn apply_run_status(paths: &RunPaths, ev: &Event) -> Result<()> {
    let mut m = match read_manifest_opt(paths)? {
        Some(m) => m,
        None => return Ok(()),
    };
    let new_status = require_status(ev, paths.events())?;
    // Terminal-state guard: a settled run never transitions again (e.g. a
    // late `run.status running` after a cancel). See run-cli-read/handoff.md D5.
    if m.status.is_terminal() {
        trace_terminal_noop(ev, m.status, new_status);
        return Ok(());
    }
    if m.status == new_status {
        return Ok(());
    }
    m.status = new_status;
    m.updated_at = ev.ts;
    write_manifest(paths, &m)
}

fn apply_node_created(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    // The envelope `node_id` is already a validated `NodeId` (parsed on read),
    // so take it directly — no re-parse needed.
    let node_id = ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=node.created missing top-level `node_id`",
            ev.seq
        ),
    })?;
    // Idempotent on replay: skip if the node already exists.
    if read_node_opt(paths, &node_id)?.is_some() {
        return Ok(());
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
        // `run_id == paths.run_id` was verified at `apply_event` entry.
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
        agent_pid: optional_i32(d, "agent_pid", &events_path, ev)?,
        agent_pid_start_time: optional_ts(d, "agent_pid_start_time", &events_path, ev)?,
        supervisor_pid: optional_i32(d, "supervisor_pid", &events_path, ev)?,
        children: Vec::new(),
        started_at: Some(ev.ts),
        updated_at: ev.ts,
        last_report: None,
        last_processed_report_seq_by_child: serde_json::Map::default(),
    };
    write_node(paths, &n)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.node_count = m.node_count.saturating_add(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_node_status(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=node.status missing top-level `node_id`",
            ev.seq
        ),
    })?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    let new_status = require_status(ev, events_path)?;
    // Terminal-state guard: a settled node never transitions again. See
    // run-cli-read/handoff.md D5.
    if n.status.is_terminal() {
        trace_terminal_noop(ev, n.status, new_status);
        return Ok(());
    }
    if n.status == new_status {
        return Ok(());
    }
    n.status = new_status;
    n.updated_at = ev.ts;
    write_node(paths, &n)
}

fn apply_node_report(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = ev.node_id.clone().ok_or_else(|| Error::CorruptEventLog {
        path: events_path.clone(),
        reason: format!(
            "event seq={} kind=node.report missing top-level `node_id`",
            ev.seq
        ),
    })?;
    let mut n = match read_node_opt(paths, &node_id)? {
        Some(n) => n,
        None => return Ok(()),
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
        return Ok(());
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
    write_node(paths, &n)
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

fn apply_discussion_opened(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let d = &ev.data;
    let discussion_id = DiscussionId::parse_str(want_str(&events_path, ev, d, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_discussion_opt(paths, &discussion_id)?.is_some() {
        return Ok(());
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
    write_discussion(paths, &disc)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.open_discussions = m.open_discussions.saturating_add(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_discussion_resolved(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let id = DiscussionId::parse_str(want_str(&events_path, ev, &ev.data, "discussion_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut disc = match read_discussion_opt(paths, &id)? {
        Some(d) => d,
        None => return Ok(()),
    };
    if matches!(disc.status, DiscussionStatus::Resolved) {
        return Ok(());
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
    write_discussion(paths, &disc)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.open_discussions = m.open_discussions.saturating_sub(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_spinoff_proposed(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let d = &ev.data;
    let proposal_id = ProposalId::parse_str(want_str(&events_path, ev, d, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    if read_spinoff_opt(paths, &proposal_id)?.is_some() {
        return Ok(());
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
    write_spinoff(paths, &s)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.pending_spinoffs = m.pending_spinoffs.saturating_add(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_spinoff_approved(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let id = ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut s = match read_spinoff_opt(paths, &id)? {
        Some(s) => s,
        None => return Ok(()),
    };
    if matches!(s.status, SpinoffStatus::Approved | SpinoffStatus::Rejected) {
        return Ok(());
    }
    s.status = SpinoffStatus::Approved;
    s.accepted_as_issue_slug = ev
        .data
        .get("issue_slug")
        .and_then(Value::as_str)
        .map(str::to_string);
    s.resolved_at = Some(ev.ts);
    write_spinoff(paths, &s)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.pending_spinoffs = m.pending_spinoffs.saturating_sub(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_spinoff_rejected(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let id = ProposalId::parse_str(want_str(&events_path, ev, &ev.data, "proposal_id")?)
        .map_err(|e| corrupt_id(&events_path, ev, &e))?;
    let mut s = match read_spinoff_opt(paths, &id)? {
        Some(s) => s,
        None => return Ok(()),
    };
    if matches!(s.status, SpinoffStatus::Approved | SpinoffStatus::Rejected) {
        return Ok(());
    }
    s.status = SpinoffStatus::Rejected;
    s.rejected_reason = ev
        .data
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    s.resolved_at = Some(ev.ts);
    write_spinoff(paths, &s)?;
    if let Some(mut m) = read_manifest_opt(paths)? {
        m.pending_spinoffs = m.pending_spinoffs.saturating_sub(1);
        m.updated_at = ev.ts;
        write_manifest(paths, &m)?;
    }
    Ok(())
}

fn apply_child_spawned(paths: &RunPaths, ev: &Event) -> Result<()> {
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
        None => return Ok(()),
    };
    let new_ref = ChildRef {
        run_id: child_run_id,
        node_id: child_node_id,
    };
    if n.children.iter().any(|c| c == &new_ref) {
        // Already recorded — pure no-op so replayed events don't churn
        // `updated_at` or the projection file.
        return Ok(());
    }
    n.children.push(new_ref);
    n.updated_at = ev.ts;
    write_node(paths, &n)
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
}
