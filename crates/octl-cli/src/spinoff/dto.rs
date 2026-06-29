//! Wire DTOs for the `spinoff` noun.
//!
//! Decouples the CLI `--json` contract from the on-disk projection
//! (`octl_core::SpinoffProposal`). The `spinoff list` handler serializes
//! a [`SpinoffSummary`] — never the projection struct — so the disk
//! schema can evolve without leaking into the public envelope.
//!
//! (`approve` / `reject` emit command-result payloads, not projection
//! serializations, so they own their wire structs directly.)
//!
//! `status` / `proposed_kind` render through the kebab helpers so a new
//! enum variant is a compile error here, not a silent wire change.

use chrono::{DateTime, Utc};
use serde::Serialize;

use octl_core::SpinoffProposal;

use crate::run::kind_kebab;
use crate::spinoff::status_kebab;

/// One row of `spinoff list --json`.
///
/// Owned: built inside the run lock from short-lived projections. The
/// kebab strings are `&'static`, so the summary carries no projection
/// borrow.
#[derive(Serialize)]
pub struct SpinoffSummary {
    pub proposal_id: String,
    pub node_id: String,
    pub status: &'static str,
    pub proposed_title: String,
    pub proposed_kind: &'static str,
    pub proposed_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_as_issue_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl From<&SpinoffProposal> for SpinoffSummary {
    fn from(s: &SpinoffProposal) -> Self {
        Self {
            proposal_id: s.proposal_id.to_string(),
            node_id: s.node_id.to_string(),
            status: status_kebab(s.status),
            proposed_title: s.proposed_title.clone(),
            proposed_kind: kind_kebab(s.proposed_kind),
            proposed_at: s.proposed_at,
            accepted_as_issue_slug: s.accepted_as_issue_slug.clone(),
            rejected_reason: s.rejected_reason.clone(),
            resolved_at: s.resolved_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::{Kind, NodeId, ProposalId, RunId, SpinoffStatus};
    use serde_json::json;

    fn ts() -> DateTime<Utc> {
        "2024-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample() -> SpinoffProposal {
        SpinoffProposal {
            schema_version: 1,
            proposal_id: ProposalId::parse_str("s-01arz3ndektsv4rrffq69g5fav").unwrap(),
            run_id: RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            node_id: NodeId::parse_str("n-0001").unwrap(),
            proposed_at: ts(),
            proposed_title: "seed proposal".to_string(),
            proposed_kind: Kind::Spinoff,
            rationale: None,
            status: SpinoffStatus::Proposed,
            accepted_as_issue_slug: None,
            rejected_reason: None,
            resolved_at: None,
        }
    }

    #[test]
    fn summary_pins_wire_shape() {
        let s = sample();
        let got = serde_json::to_value(SpinoffSummary::from(&s)).unwrap();
        assert_eq!(
            got,
            json!({
                "proposal_id": "s-01arz3ndektsv4rrffq69g5fav",
                "node_id": "n-0001",
                "status": "pending",
                "proposed_title": "seed proposal",
                "proposed_kind": "spinoff",
                "proposed_at": "2024-01-01T00:00:00Z",
            })
        );
    }
}
