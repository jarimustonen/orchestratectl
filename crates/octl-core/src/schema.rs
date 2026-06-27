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
    /// Interactive, human-reviewed coding worktree (`/worktree-code`).
    Code,
    /// Autonomous fire-and-forget task that merges itself back (`/worktree-spinoff`).
    Spinoff,
    /// Orchestrated worker reporting to an orchestrator (`/worktree-orchestrated`).
    Orchestrated,
    /// Autonomous multi-source research worktree (`/worktree-research`).
    Research,
    /// Drives one architectural decision to an ADR (`/worktree-technical-decision`).
    TechnicalDecision,
    /// Authors a new Claude Code skill (`/worktree-make-skill`).
    MakeSkill,
    /// Parallel fan-out of many identical units (`/fan-out`).
    FanOut,
    /// End-to-end bug investigate-fix-review worktree (`/worktree-bugfix`).
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
    /// Agent runs to completion unattended; the watchdog adjudicates exit.
    Autonomous,
    /// Human-driven inside a tmux window; no watchdog-forced termination.
    Interactive,
}

/// Run/node status (design.md §1.2).
///
/// `Done`, `Failed`, and `Cancelled` are **terminal**: once a run or node
/// reaches one of them its `status` must never change again. The reducer
/// enforces this — `apply_run_status`, `apply_node_status`, and
/// `apply_node_report` are all no-ops once [`Status::is_terminal`] holds — so
/// a late-arriving event (e.g. an agent success report racing a `run cancel`)
/// cannot resurrect a settled state. Only the `status` field is frozen;
/// other projection fields may still be mutated by non-status events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Created but not yet started.
    Pending,
    /// Actively executing.
    Running,
    /// Stalled awaiting input (e.g. an open discussion).
    Blocked,
    /// Completed successfully (terminal).
    Done,
    /// Completed with failure (terminal).
    Failed,
    /// Terminated before completion by an operator or parent (terminal).
    Cancelled,
}

impl Status {
    /// True for the terminal states `Done | Failed | Cancelled`. A run or
    /// node in a terminal state is settled: the reducer treats any further
    /// *status* transition as a no-op. "Settled" applies to `status` only —
    /// non-status projection fields (e.g. `Node::children` via
    /// `child.spawned`, or manifest counters) can still change.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Done | Status::Failed | Status::Cancelled)
    }
}

/// Discussion lifecycle status (design.md §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiscussionStatus {
    /// Awaiting a decision.
    Open,
    /// A choice has been recorded; the run may proceed.
    Resolved,
}

/// Spin-off proposal status (design.md §1.5, §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpinoffStatus {
    /// Suggested by an agent, awaiting human triage.
    Proposed,
    /// Accepted; typically promoted to a tracked issue.
    Approved,
    /// Declined, with a reason recorded.
    Rejected,
}

/// `manifest.json` (design.md §1.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// State-schema version this file was written with.
    pub schema_version: u32,
    /// Unique run identifier (ULID).
    pub run_id: String,
    /// Kind of work this run performs.
    pub kind: Kind,
    /// Execution lifecycle (autonomous vs interactive).
    pub lifecycle: Lifecycle,
    /// Human-readable run title.
    pub title: String,
    /// Current aggregate run status.
    pub status: Status,
    /// When the run was created.
    pub created_at: DateTime<Utc>,
    /// When the manifest was last modified.
    pub updated_at: DateTime<Utc>,
    /// Source repository the run operates on, if any.
    pub source_repo: Option<String>,
    /// Branch the run was started from, if any.
    pub source_branch: Option<String>,
    /// Root directory under which this run's worktrees live, if any.
    pub worktree_root: Option<String>,
    /// Number of nodes created in this run (denormalized counter).
    pub node_count: u32,
    /// Count of currently open discussions (denormalized counter).
    pub open_discussions: u32,
    /// Count of currently pending spin-off proposals (denormalized counter).
    pub pending_spinoffs: u32,
    /// Run that spawned this run, if it is itself a child.
    pub parent_run_id: Option<String>,
    /// Node in the parent run that spawned this run, if any.
    pub parent_node_id: Option<String>,
}

/// `(child_run_id, child_node_id)` pointer recorded in `Node::children`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildRef {
    /// Run id of the spawned child.
    pub run_id: String,
    /// Node id within the child run.
    pub node_id: String,
}

/// `nodes/<node-id>.json` (design.md §1.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// State-schema version this file was written with.
    pub schema_version: u32,
    /// Unique node identifier within its run (e.g. `n-0001`).
    pub node_id: String,
    /// Run this node belongs to.
    pub run_id: String,
    /// Parent node within the same run, if this is a sub-node.
    pub parent_node_id: Option<String>,
    /// Kind of work this node performs.
    pub kind: Kind,
    /// Current node status.
    pub status: Status,
    /// Task description / prompt driving the node, if recorded.
    pub task: Option<String>,
    /// Filesystem path of the node's git worktree, if created.
    pub worktree_path: Option<String>,
    /// Git branch the node works on, if any.
    pub branch: Option<String>,
    /// tmux window hosting the node's agent, if interactive.
    pub tmux_window: Option<String>,
    /// PID of the running agent process, if live.
    pub agent_pid: Option<i32>,
    /// Start time of the agent process, used to detect PID reuse.
    pub agent_pid_start_time: Option<DateTime<Utc>>,
    /// PID of the supervisor watching this node, if live.
    pub supervisor_pid: Option<i32>,
    /// Children this node has spawned.
    #[serde(default)]
    pub children: Vec<ChildRef>,
    /// When the node started executing, if it has.
    pub started_at: Option<DateTime<Utc>>,
    /// When the node file was last modified.
    pub updated_at: DateTime<Utc>,
    /// The `node.report` payload that drove this node to its terminal status.
    /// Set only by the report that actually transitions the node (Done /
    /// Failed / Cancelled). Once the node is terminal it is frozen: a late
    /// report against an already-settled node is dropped without overwriting
    /// this field (see `reducer::apply_node_report`). So for a node cancelled
    /// by `run cancel`, this holds the synthesized cancel report, not a
    /// later-arriving agent report — that payload remains only in
    /// `events.jsonl`.
    pub last_report: Option<Value>,
    /// Highest report `seq` consumed per child run id, for idempotent
    /// report processing across supervisor restarts.
    #[serde(default)]
    pub last_processed_report_seq_by_child: Map<String, Value>,
}

/// `discussions/<discussion-id>.json` (design.md §1.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    /// State-schema version this file was written with.
    pub schema_version: u32,
    /// Unique discussion identifier.
    pub discussion_id: String,
    /// Run this discussion belongs to.
    pub run_id: String,
    /// Node that opened the discussion.
    pub node_id: String,
    /// When the discussion was opened.
    pub opened_at: DateTime<Utc>,
    /// Severity tag (e.g. `critical`, `normal`) driving alerting.
    pub severity: String,
    /// Short summary of what needs deciding.
    pub topic: String,
    /// Optional longer context for the decision.
    pub context: Option<String>,
    /// Candidate choices offered to the resolver.
    #[serde(default)]
    pub options: Vec<String>,
    /// Open vs resolved.
    pub status: DiscussionStatus,
    /// The chosen resolution, once resolved.
    pub resolution: Option<String>,
    /// Free-form note accompanying the resolution.
    #[serde(default)]
    pub note: Option<String>,
    /// When the discussion was resolved, if it has been.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// `spinoffs/<proposal-id>.json` (design.md §1.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpinoffProposal {
    /// State-schema version this file was written with.
    pub schema_version: u32,
    /// Unique proposal identifier.
    pub proposal_id: String,
    /// Run this proposal belongs to.
    pub run_id: String,
    /// Node that proposed the spin-off.
    pub node_id: String,
    /// When the proposal was made.
    pub proposed_at: DateTime<Utc>,
    /// Suggested title for the spun-off work.
    pub proposed_title: String,
    /// Suggested kind for the spun-off run.
    pub proposed_kind: Kind,
    /// Why the agent proposed this spin-off.
    pub rationale: Option<String>,
    /// Proposed / approved / rejected.
    pub status: SpinoffStatus,
    /// Issue slug the proposal was promoted to, once approved.
    pub accepted_as_issue_slug: Option<String>,
    /// Reason recorded when the proposal is rejected.
    pub rejected_reason: Option<String>,
    /// When the proposal was approved or rejected, if it has been.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// One event-log line (design.md §1.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Wall-clock timestamp the event was appended.
    pub ts: DateTime<Utc>,
    /// Monotonic per-run sequence number (recovered on append).
    pub seq: u64,
    /// Event kind discriminator (e.g. `node.created`, `discussion.opened`).
    pub kind: String,
    /// Run the event belongs to.
    pub run_id: String,
    /// Node the event concerns, when applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub node_id: Option<String>,
    /// Caller-supplied key used to dedupe retried appends.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<String>,
    /// Kind-specific payload applied by the reducer.
    #[serde(default)]
    pub data: Value,
}
