//! On-disk state schema types per `design.md` §1.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The current state-on-disk schema version this crate writes.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// All state-schema versions this crate can read.
pub const SUPPORTED_STATE_SCHEMAS: &[u32] = &[1];

/// The run/node kind enum (design.md §1.2).
///
/// All 8 kinds are active in MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Code,
    Spinoff,
    Orchestrated,
    Research,
    TechnicalDecision,
    MakeSkill,
    FanOut,
    Bugfix,
}

impl Kind {
    /// Default lifecycle for a kind (design.md §7.4). `code` is
    /// interactive (human-driven inside tmux); every other MVP kind is
    /// autonomous (agent runs to completion, watchdog adjudicates).
    pub fn lifecycle(self) -> Lifecycle {
        match self {
            Kind::Code => Lifecycle::Interactive,
            Kind::Spinoff
            | Kind::Orchestrated
            | Kind::Research
            | Kind::TechnicalDecision
            | Kind::MakeSkill
            | Kind::FanOut
            | Kind::Bugfix => Lifecycle::Autonomous,
        }
    }
}

/// Lifecycle (design.md §1.2, §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Autonomous,
    Interactive,
}

/// Run/node status (design.md §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pending,
    Running,
    Blocked,
    Done,
    Failed,
    Cancelled,
}

/// Discussion lifecycle status (design.md §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiscussionStatus {
    Open,
    Resolved,
}

/// Spin-off proposal status (design.md §1.5, §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpinoffStatus {
    Proposed,
    Approved,
    Rejected,
}

/// `manifest.json` (design.md §1.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub run_id: String,
    pub kind: Kind,
    pub lifecycle: Lifecycle,
    pub title: String,
    pub status: Status,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub worktree_root: Option<String>,
    pub node_count: u32,
    pub open_discussions: u32,
    pub pending_spinoffs: u32,
    pub parent_run_id: Option<String>,
    pub parent_node_id: Option<String>,
}

/// `(child_run_id, child_node_id)` pointer recorded in `Node::children`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildRef {
    pub run_id: String,
    pub node_id: String,
}

/// `nodes/<node-id>.json` (design.md §1.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub schema_version: u32,
    pub node_id: String,
    pub run_id: String,
    pub parent_node_id: Option<String>,
    pub kind: Kind,
    pub status: Status,
    pub task: Option<String>,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub tmux_window: Option<String>,
    pub agent_pid: Option<i32>,
    pub agent_pid_start_time: Option<DateTime<Utc>>,
    pub supervisor_pid: Option<i32>,
    #[serde(default)]
    pub children: Vec<ChildRef>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub last_report: Option<Value>,
    #[serde(default)]
    pub last_processed_report_seq_by_child: Map<String, Value>,
}

/// `discussions/<discussion-id>.json` (design.md §1.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    pub schema_version: u32,
    pub discussion_id: String,
    pub run_id: String,
    pub node_id: String,
    pub opened_at: DateTime<Utc>,
    pub severity: String,
    pub topic: String,
    pub context: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    pub status: DiscussionStatus,
    pub resolution: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// `spinoffs/<proposal-id>.json` (design.md §1.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpinoffProposal {
    pub schema_version: u32,
    pub proposal_id: String,
    pub run_id: String,
    pub node_id: String,
    pub proposed_at: DateTime<Utc>,
    pub proposed_title: String,
    pub proposed_kind: Kind,
    pub rationale: Option<String>,
    pub status: SpinoffStatus,
    pub accepted_as_issue_slug: Option<String>,
    pub rejected_reason: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// One event-log line (design.md §1.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub ts: DateTime<Utc>,
    pub seq: u64,
    pub kind: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub data: Value,
}
