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
    ChildRef, Discussion, DiscussionStatus, Event, Kind, Lifecycle, Manifest, Node,
    SpinoffProposal, SpinoffStatus, Status, STATE_SCHEMA_VERSION,
};

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
pub fn apply_event(paths: &RunPaths, ev: &Event) -> Result<()> {
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
        run_id: ev.run_id.clone(),
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
        parent_run_id: d
            .get("parent_run_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        parent_node_id: d
            .get("parent_node_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    write_manifest(paths, &m)
}

fn apply_run_status(paths: &RunPaths, ev: &Event) -> Result<()> {
    let mut m = match read_manifest_opt(paths)? {
        Some(m) => m,
        None => return Ok(()),
    };
    let new_status = require_status(ev, paths.events())?;
    if m.status == new_status {
        return Ok(());
    }
    m.status = new_status;
    m.updated_at = ev.ts;
    write_manifest(paths, &m)
}

fn apply_node_created(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=node.created missing top-level `node_id`",
                ev.seq
            ),
        })?
        .to_string();
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
        node_id: node_id.clone(),
        run_id: ev.run_id.clone(),
        parent_node_id: d
            .get("parent_node_id")
            .and_then(Value::as_str)
            .map(str::to_string),
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
        last_processed_report_seq_by_child: Default::default(),
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
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=node.status missing top-level `node_id`",
                ev.seq
            ),
        })?;
    let mut n = match read_node_opt(paths, node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    let new_status = require_status(ev, events_path)?;
    if n.status == new_status {
        return Ok(());
    }
    n.status = new_status;
    n.updated_at = ev.ts;
    write_node(paths, &n)
}

fn apply_node_report(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path,
            reason: format!(
                "event seq={} kind=node.report missing top-level `node_id`",
                ev.seq
            ),
        })?;
    let mut n = match read_node_opt(paths, node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    n.last_report = Some(ev.data.clone());
    n.updated_at = ev.ts;
    // Cancellation is reported via `{cancelled: true}` regardless of
    // whether `success` is present (a synthesized cancel-report from
    // `run cancel` may omit `success`). See design.md §7.7.
    let cancelled = ev
        .data
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if cancelled {
        n.status = Status::Cancelled;
    } else if let Some(success) = ev.data.get("success").and_then(Value::as_bool) {
        n.status = if success {
            Status::Done
        } else {
            Status::Failed
        };
    }
    write_node(paths, &n)
}

fn apply_discussion_opened(paths: &RunPaths, ev: &Event) -> Result<()> {
    let events_path = paths.events();
    let d = &ev.data;
    let discussion_id = want_str(&events_path, ev, d, "discussion_id")?.to_string();
    if read_discussion_opt(paths, &discussion_id)?.is_some() {
        return Ok(());
    }
    let node_id = d
        .get("node_id")
        .and_then(Value::as_str)
        .or(ev.node_id.as_deref())
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=discussion.opened missing `node_id`",
                ev.seq
            ),
        })?
        .to_string();
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
        run_id: ev.run_id.clone(),
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
    let id = want_str(&events_path, ev, &ev.data, "discussion_id")?;
    let mut disc = match read_discussion_opt(paths, id)? {
        Some(d) => d,
        None => return Ok(()),
    };
    if matches!(disc.status, DiscussionStatus::Resolved) {
        return Ok(());
    }
    disc.status = DiscussionStatus::Resolved;
    disc.resolution = ev
        .data
        .get("resolution")
        .and_then(Value::as_str)
        .map(str::to_string);
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
    let proposal_id = want_str(&events_path, ev, d, "proposal_id")?.to_string();
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
    let node_id = d
        .get("node_id")
        .and_then(Value::as_str)
        .or(ev.node_id.as_deref())
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=spinoff.proposed missing `node_id`",
                ev.seq
            ),
        })?
        .to_string();
    let s = SpinoffProposal {
        schema_version: STATE_SCHEMA_VERSION,
        proposal_id,
        run_id: ev.run_id.clone(),
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
    let id = want_str(&events_path, ev, &ev.data, "proposal_id")?;
    let mut s = match read_spinoff_opt(paths, id)? {
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
    let id = want_str(&events_path, ev, &ev.data, "proposal_id")?;
    let mut s = match read_spinoff_opt(paths, id)? {
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
    let parent_node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.clone(),
            reason: format!(
                "event seq={} kind=child.spawned missing parent `node_id`",
                ev.seq
            ),
        })?;
    let child_run_id = want_str(&events_path, ev, &ev.data, "child_run_id")?.to_string();
    let child_node_id = ev
        .data
        .get("child_node_id")
        .and_then(Value::as_str)
        .unwrap_or("n-0001")
        .to_string();
    let mut n = match read_node_opt(paths, parent_node_id)? {
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
