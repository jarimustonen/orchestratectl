//! Event → projection reducer (design.md §1.4).
//!
//! Each event mutates zero or more projection files. Unknown kinds are
//! ignored for forward compatibility. The reducer expects to run under the
//! per-run `flock`.

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

fn parse_ts(v: &Value, fallback: DateTime<Utc>) -> DateTime<Utc> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(fallback)
}

fn data_kind(v: &Value) -> Option<Kind> {
    serde_json::from_value(v.clone()).ok()
}

fn data_status(v: &Value) -> Option<Status> {
    serde_json::from_value(v.clone()).ok()
}

fn want_str<'a>(d: &'a Value, field: &str, kind: &str, ev_kind: &str) -> Result<&'a str> {
    d.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::CorruptEventLog {
            path: format!("event#{kind}").into(),
            reason: format!("{ev_kind} missing `{field}` string field"),
        })
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
    let d = &ev.data;
    let kind =
        data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
            path: paths.manifest(),
            reason: "run.created missing/invalid `kind`".into(),
        })?;
    let lifecycle: Lifecycle = serde_json::from_value(
        d.get("lifecycle").cloned().unwrap_or(Value::Null),
    )
    .map_err(|_| Error::CorruptEventLog {
        path: paths.manifest(),
        reason: "run.created missing/invalid `lifecycle`".into(),
    })?;
    let title = want_str(d, "title", "manifest", "run.created")?.to_string();
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
    if let Some(s) = data_status(ev.data.get("status").unwrap_or(&Value::Null)) {
        m.status = s;
    }
    m.updated_at = ev.ts;
    write_manifest(paths, &m)
}

fn apply_node_created(paths: &RunPaths, ev: &Event) -> Result<()> {
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.root.clone(),
            reason: "node.created missing top-level `node_id`".into(),
        })?
        .to_string();
    let d = &ev.data;
    let kind =
        data_kind(d.get("kind").unwrap_or(&Value::Null)).ok_or_else(|| Error::CorruptEventLog {
            path: paths.node(&node_id),
            reason: "node.created missing/invalid `kind`".into(),
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
        agent_pid: d.get("agent_pid").and_then(Value::as_i64).map(|v| v as i32),
        agent_pid_start_time: d
            .get("agent_pid_start_time")
            .map(|v| parse_ts(v, ev.ts))
            .filter(|_| d.get("agent_pid_start_time").is_some()),
        supervisor_pid: d
            .get("supervisor_pid")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
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
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.root.clone(),
            reason: "node.status missing top-level `node_id`".into(),
        })?;
    let mut n = match read_node_opt(paths, node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    if let Some(s) = data_status(ev.data.get("status").unwrap_or(&Value::Null)) {
        n.status = s;
    }
    n.updated_at = ev.ts;
    write_node(paths, &n)
}

fn apply_node_report(paths: &RunPaths, ev: &Event) -> Result<()> {
    let node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.root.clone(),
            reason: "node.report missing top-level `node_id`".into(),
        })?;
    let mut n = match read_node_opt(paths, node_id)? {
        Some(n) => n,
        None => return Ok(()),
    };
    n.last_report = Some(ev.data.clone());
    n.updated_at = ev.ts;
    // Terminal report → derive node status.
    if let Some(success) = ev.data.get("success").and_then(Value::as_bool) {
        n.status = if success {
            Status::Done
        } else {
            Status::Failed
        };
        if ev
            .data
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            n.status = Status::Cancelled;
        }
    }
    write_node(paths, &n)
}

fn apply_discussion_opened(paths: &RunPaths, ev: &Event) -> Result<()> {
    let d = &ev.data;
    let discussion_id =
        want_str(d, "discussion_id", "discussion", "discussion.opened")?.to_string();
    if read_discussion_opt(paths, &discussion_id)?.is_some() {
        // Idempotent: already exists, skip.
        return Ok(());
    }
    let node_id = d
        .get("node_id")
        .and_then(Value::as_str)
        .or(ev.node_id.as_deref())
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.discussion(&discussion_id),
            reason: "discussion.opened missing `node_id`".into(),
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
        topic: want_str(d, "topic", "discussion", "discussion.opened")?.to_string(),
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
    let id = want_str(
        &ev.data,
        "discussion_id",
        "discussion",
        "discussion.resolved",
    )?;
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
    let d = &ev.data;
    let proposal_id = want_str(d, "proposal_id", "spinoff", "spinoff.proposed")?.to_string();
    if read_spinoff_opt(paths, &proposal_id)?.is_some() {
        return Ok(());
    }
    let proposed_kind =
        data_kind(d.get("proposed_kind").unwrap_or(&Value::Null)).ok_or_else(|| {
            Error::CorruptEventLog {
                path: paths.spinoff(&proposal_id),
                reason: "spinoff.proposed missing/invalid `proposed_kind`".into(),
            }
        })?;
    let node_id = d
        .get("node_id")
        .and_then(Value::as_str)
        .or(ev.node_id.as_deref())
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.spinoff(&proposal_id),
            reason: "spinoff.proposed missing `node_id`".into(),
        })?
        .to_string();
    let s = SpinoffProposal {
        schema_version: STATE_SCHEMA_VERSION,
        proposal_id,
        run_id: ev.run_id.clone(),
        node_id,
        proposed_at: ev.ts,
        proposed_title: want_str(d, "proposed_title", "spinoff", "spinoff.proposed")?.to_string(),
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
    let id = want_str(&ev.data, "proposal_id", "spinoff", "spinoff.approved")?;
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
    let id = want_str(&ev.data, "proposal_id", "spinoff", "spinoff.rejected")?;
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
    let parent_node_id = ev
        .node_id
        .as_deref()
        .ok_or_else(|| Error::CorruptEventLog {
            path: paths.root.clone(),
            reason: "child.spawned missing parent `node_id`".into(),
        })?;
    let child_run_id = want_str(&ev.data, "child_run_id", "node", "child.spawned")?.to_string();
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
    if !n.children.iter().any(|c| c == &new_ref) {
        n.children.push(new_ref);
    }
    n.updated_at = ev.ts;
    write_node(paths, &n)
}
