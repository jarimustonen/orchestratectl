//! Wire DTOs for the `discussion` noun.
//!
//! These decouple the CLI `--json` contract from the on-disk projection
//! (`octl_core::Discussion`). Handlers serialize a `*View` — never the
//! projection struct — so the disk schema can gain, rename, or split
//! fields without leaking into the public envelope. Only the fields
//! listed here are part of the wire contract.
//!
//! `status` is rendered through [`status_kebab`] rather than serde's enum
//! rename so adding a `DiscussionStatus` variant is a compile error here,
//! not a silent wire change.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::{Discussion, DiscussionId, NodeId, RunId};

use super::status_kebab;

/// Full single-discussion wire view (`discussion show --json`).
///
/// Borrows from the projection: the `show` handler holds the `Discussion`
/// for the lifetime of the emit, so no clone is needed. Field order and
/// names mirror the established wire contract exactly (every field is
/// emitted, including explicit `null`s).
#[derive(Serialize)]
pub struct DiscussionView<'a> {
    pub schema_version: u32,
    pub discussion_id: &'a DiscussionId,
    pub run_id: &'a RunId,
    pub node_id: &'a NodeId,
    pub opened_at: DateTime<Utc>,
    pub severity: &'a str,
    pub topic: &'a str,
    pub context: Option<&'a str>,
    pub options: &'a [String],
    pub status: &'static str,
    pub resolution: Option<&'a str>,
    pub note: Option<&'a str>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl<'a> From<&'a Discussion> for DiscussionView<'a> {
    fn from(d: &'a Discussion) -> Self {
        Self {
            schema_version: d.schema_version,
            discussion_id: &d.discussion_id,
            run_id: &d.run_id,
            node_id: &d.node_id,
            opened_at: d.opened_at,
            severity: &d.severity,
            topic: &d.topic,
            context: d.context.as_deref(),
            options: &d.options,
            status: status_kebab(d.status),
            resolution: d.resolution.as_deref(),
            note: d.note.as_deref(),
            resolved_at: d.resolved_at,
        }
    }
}

/// One row of `discussion list --json`.
///
/// Owned (not borrowing): the list handler builds these inside the run
/// lock from short-lived projections that are dropped before output is
/// formatted, so the summary cannot hold a reference into them.
#[derive(Serialize)]
pub struct DiscussionSummary {
    pub discussion_id: String,
    pub node_id: String,
    pub status: String,
    pub severity: String,
    pub topic: String,
    pub opened_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl From<&Discussion> for DiscussionSummary {
    fn from(d: &Discussion) -> Self {
        Self {
            discussion_id: d.discussion_id.to_string(),
            node_id: d.node_id.to_string(),
            status: status_kebab(d.status).to_string(),
            severity: d.severity.clone(),
            topic: d.topic.clone(),
            opened_at: d.opened_at,
            resolved_at: d.resolved_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::DiscussionStatus;
    use serde_json::json;

    fn ts() -> DateTime<Utc> {
        "2024-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample() -> Discussion {
        Discussion {
            schema_version: 1,
            discussion_id: DiscussionId::parse_str("d-pqrstuvwxy").unwrap(),
            run_id: RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            node_id: NodeId::parse_str("n-0001").unwrap(),
            opened_at: ts(),
            severity: "discuss".to_string(),
            topic: "seed topic".to_string(),
            context: None,
            options: Vec::new(),
            status: DiscussionStatus::Open,
            resolution: None,
            note: None,
            resolved_at: None,
        }
    }

    #[test]
    fn view_pins_wire_shape() {
        let d = sample();
        let got = serde_json::to_value(DiscussionView::from(&d)).unwrap();
        assert_eq!(
            got,
            json!({
                "schema_version": 1,
                "discussion_id": "d-pqrstuvwxy",
                "run_id": "01arz3ndektsv4rrffq69g5fav",
                "node_id": "n-0001",
                "opened_at": "2024-01-01T00:00:00Z",
                "severity": "discuss",
                "topic": "seed topic",
                "context": null,
                "options": [],
                "status": "open",
                "resolution": null,
                "note": null,
                "resolved_at": null,
            })
        );
    }

    #[test]
    fn summary_pins_wire_shape() {
        let d = sample();
        let got = serde_json::to_value(DiscussionSummary::from(&d)).unwrap();
        assert_eq!(
            got,
            json!({
                "discussion_id": "d-pqrstuvwxy",
                "node_id": "n-0001",
                "status": "open",
                "severity": "discuss",
                "topic": "seed topic",
                "opened_at": "2024-01-01T00:00:00Z",
            })
        );
    }
}
