//! On-disk state schema types per `design.md` §1.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The current state-on-disk schema version this crate writes.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// All state-schema versions this crate can read.
pub const SUPPORTED_STATE_SCHEMAS: &[u32] = &[1];

/// Crockford base32 alphabet in lowercase (excludes `i`, `l`, `o`, `u`). The
/// charset for the bare ULID of a [`RunId`].
const CROCKFORD_LOWER: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// True iff every byte of `s` is a lowercase Crockford base32 character.
fn all_crockford_lower(s: &str) -> bool {
    s.bytes().all(|b| CROCKFORD_LOWER.contains(&b))
}

/// True iff every byte of `s` is a lowercase ASCII alphanumeric (`a-z0-9`).
///
/// This is the charset for the body of a [`DiscussionId`] / [`ProposalId`].
/// It is deliberately wider than Crockford: those ids come in two on-disk
/// flavours — `x-<ulid>` (Crockford base32) *and* `x-<sha-prefix>`, the
/// deterministic-id form emitted by the supervisor, which is RFC 4648 base32
/// lowercase (`a-z2-7`, so it can contain `i`/`l`/`o`/`u`). A Crockford-only
/// charset would reject every supervisor-generated discussion/spinoff id.
/// `a-z0-9` accepts both while still excluding `/`, `.`, `-`, and uppercase —
/// so the path-traversal guard is unaffected.
fn all_lower_alnum(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Error returned when a typed identifier fails parse-time validation.
///
/// Every [`RunId`], [`NodeId`], [`DiscussionId`], and [`ProposalId`] is
/// constructed only through its `parse_str` constructor (or the equivalent
/// validating `Deserialize`), so any value that reaches a path helper has
/// already been checked for prefix, charset, and length. This is the
/// path-traversal guard: a raw id containing `/`, `..`, or a leading dot can
/// never be turned into one of these newtypes, so it can never name a file
/// outside the run directory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdValidationError {
    /// The value carried the right prefix (or needs none) but its body had the
    /// wrong length or used characters outside the permitted charset.
    #[error("invalid {kind} id {value:?}: expected {expected}")]
    InvalidFormat {
        /// Which id type rejected the value (`run`, `node`, `discussion`, `spinoff`).
        kind: &'static str,
        /// The offending raw value.
        value: String,
        /// Human-readable description of the accepted shape (e.g. `n-NNNN`).
        expected: &'static str,
    },
    /// The value did not start with the id type's required prefix
    /// (`n-`, `d-`, `s-`).
    #[error("invalid {kind} id: wrong prefix, expected {expected}")]
    WrongPrefix {
        /// Which id type rejected the value.
        kind: &'static str,
        /// Human-readable description of the accepted shape.
        expected: &'static str,
    },
}

impl IdValidationError {
    /// The id type that rejected the value (`run`, `node`, `discussion`, `spinoff`).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidFormat { kind, .. } | Self::WrongPrefix { kind, .. } => kind,
        }
    }

    /// The accepted-shape hint, suitable for the `expected` field of a CLI
    /// error envelope.
    pub fn expected(&self) -> &'static str {
        match self {
            Self::InvalidFormat { expected, .. } | Self::WrongPrefix { expected, .. } => expected,
        }
    }
}

/// Generate the shared trait surface for a validated id newtype: `as_str`,
/// `Display`, `Debug`, `Serialize` (as the bare string), and a validating
/// `Deserialize` (delegates to `parse_str`, so reading an old file with a
/// malformed id fails loudly rather than silently widening the type). Each
/// newtype supplies its own `parse_str` in a separate `impl` block.
macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// The validated id as a string slice. There is no mutable or
            /// owned-`String` accessor by design: the inner value can never be
            /// mutated into an unvalidated state after construction.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse_str(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

id_newtype! {
    /// A validated run identifier: a lowercase ULID (26 Crockford base32
    /// characters whose first character keeps the encoded timestamp within
    /// ULID's 48-bit range). Mirrors what [`crate::new_run_id`] emits.
    RunId
}

impl RunId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str = "26-char lowercase Crockford base32 ULID";
    /// Canonical length of a ULID in Crockford base32.
    const LEN: usize = 26;

    /// Parse and validate a `run_id`. Accepts only the 26-character lowercase
    /// ULID shape; rejects wrong length, non-Crockford characters, and a first
    /// character outside `0..=7` (which would overflow ULID's 48-bit timestamp).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let reject = || IdValidationError::InvalidFormat {
            kind: "run",
            value: s.to_string(),
            expected: Self::EXPECTED,
        };
        if s.len() != Self::LEN || !all_crockford_lower(s) {
            return Err(reject());
        }
        // The first base32 char carries the top 5 bits of the 128-bit ULID;
        // the 48-bit timestamp cannot overflow only if it is in `0..=7`.
        if !(b'0'..=b'7').contains(&s.as_bytes()[0]) {
            return Err(reject());
        }
        Ok(Self(s.to_string()))
    }
}

id_newtype! {
    /// A validated node identifier: `n-` followed by 4 or more ASCII digits
    /// (e.g. `n-0001`). Mirrors what [`crate::format_node_id`] emits.
    NodeId
}

impl NodeId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str = "n-NNNN (n- followed by 4+ ASCII digits)";

    /// Parse and validate a `node_id`. Requires the `n-` prefix followed by at
    /// least four ASCII digits; rejects anything else (wrong prefix, too few
    /// digits, non-digit body).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("n-").ok_or(IdValidationError::WrongPrefix {
            kind: "node",
            expected: Self::EXPECTED,
        })?;
        if body.len() >= 4 && body.bytes().all(|b| b.is_ascii_digit()) {
            Ok(Self(s.to_string()))
        } else {
            Err(IdValidationError::InvalidFormat {
                kind: "node",
                value: s.to_string(),
                expected: Self::EXPECTED,
            })
        }
    }
}

id_newtype! {
    /// A validated discussion identifier: `d-` followed by 10–26 lowercase
    /// ASCII alphanumeric characters. Covers both the `d-<ulid>` form
    /// (26 chars, Crockford base32) and the shorter `d-<sha-prefix>`
    /// deterministic-id form (RFC 4648 base32 lowercase, `a-z2-7`).
    DiscussionId
}

impl DiscussionId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str = "d-<10-26 lowercase alphanumeric chars>";

    /// Parse and validate a `discussion_id`. Requires the `d-` prefix followed
    /// by 10–26 lowercase ASCII alphanumeric characters (see [`all_lower_alnum`]
    /// for why the charset is wider than Crockford).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("d-").ok_or(IdValidationError::WrongPrefix {
            kind: "discussion",
            expected: Self::EXPECTED,
        })?;
        if (10..=26).contains(&body.len()) && all_lower_alnum(body) {
            Ok(Self(s.to_string()))
        } else {
            Err(IdValidationError::InvalidFormat {
                kind: "discussion",
                value: s.to_string(),
                expected: Self::EXPECTED,
            })
        }
    }
}

id_newtype! {
    /// A validated spin-off proposal identifier: `s-` followed by 10–26
    /// lowercase ASCII alphanumeric characters. Covers both the `s-<ulid>` form
    /// (26 chars, Crockford base32) and the shorter `s-<sha-prefix>`
    /// deterministic-id form (RFC 4648 base32 lowercase, `a-z2-7`).
    ProposalId
}

impl ProposalId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str = "s-<10-26 lowercase alphanumeric chars>";

    /// Parse and validate a `proposal_id`. Requires the `s-` prefix followed
    /// by 10–26 lowercase ASCII alphanumeric characters (see [`all_lower_alnum`]
    /// for why the charset is wider than Crockford).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("s-").ok_or(IdValidationError::WrongPrefix {
            kind: "spinoff",
            expected: Self::EXPECTED,
        })?;
        if (10..=26).contains(&body.len()) && all_lower_alnum(body) {
            Ok(Self(s.to_string()))
        } else {
            Err(IdValidationError::InvalidFormat {
                kind: "spinoff",
                value: s.to_string(),
                expected: Self::EXPECTED,
            })
        }
    }
}

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
    /// The kebab-case wire name for this kind — the same string serde
    /// (de)serializes via `rename_all = "kebab-case"`.
    ///
    /// The exhaustive `match` is deliberate: adding a `Kind` variant fails
    /// to compile until its wire name is listed here, so [`Kind::WIRE_NAMES`]
    /// and any caller that advertises the accepted kinds (e.g. the report
    /// validator's `expected` hint) cannot silently drift from the enum.
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Kind::Code => "code",
            Kind::Spinoff => "spinoff",
            Kind::Orchestrated => "orchestrated",
            Kind::Research => "research",
            Kind::TechnicalDecision => "technical-decision",
            Kind::MakeSkill => "make-skill",
            Kind::FanOut => "fan-out",
            Kind::Bugfix => "bugfix",
        }
    }

    /// Every kind's kebab-case wire name, in declaration order. Single
    /// source of truth for "the set of accepted kinds" — see [`Kind::wire_name`].
    pub const WIRE_NAMES: &'static [&'static str] = &[
        Kind::Code.wire_name(),
        Kind::Spinoff.wire_name(),
        Kind::Orchestrated.wire_name(),
        Kind::Research.wire_name(),
        Kind::TechnicalDecision.wire_name(),
        Kind::MakeSkill.wire_name(),
        Kind::FanOut.wire_name(),
        Kind::Bugfix.wire_name(),
    ];

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
    /// Unique node identifier within its run (e.g. `n-0001`). Validated on
    /// read; this is the projection's filename key, so it can never name a
    /// path outside `nodes/`.
    pub node_id: NodeId,
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
    /// Unique discussion identifier. Validated on read; this is the
    /// projection's filename key, so it can never name a path outside
    /// `discussions/`.
    pub discussion_id: DiscussionId,
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
    /// Unique proposal identifier. Validated on read; this is the projection's
    /// filename key, so it can never name a path outside `spinoffs/`.
    pub proposal_id: ProposalId,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `Kind::wire_name` (and thus `Kind::WIRE_NAMES`) must stay identical
    /// to what serde actually (de)serializes. If the `rename_all` routing
    /// or a variant name ever diverges from `wire_name`, this fails — which
    /// is what keeps the report validator's `expected` hint honest.
    #[test]
    fn wire_names_match_serde_round_trip() {
        for &name in Kind::WIRE_NAMES {
            let kind: Kind = serde_json::from_value(Value::String(name.to_string()))
                .unwrap_or_else(|_| panic!("WIRE_NAMES entry {name:?} is not a valid Kind"));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                Value::String(name.to_string()),
                "serde round-trip diverged from wire_name for {name:?}",
            );
        }
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;

    /// Inputs every id type must reject — the path-traversal vectors plus the
    /// generic malformed cases called out in the issue's success criteria.
    const TRAVERSAL_VECTORS: &[&str] = &[
        "..",
        "../etc",
        "a/b",
        "a/../b",
        ".hidden",
        "./x",
        "foo/bar.json",
        "n-0001/../../etc",
        "",
    ];

    #[test]
    fn run_id_accepts_generator_output_and_rejects_malformed() {
        let id = crate::new_run_id();
        assert!(
            RunId::parse_str(&id).is_ok(),
            "generator must validate: {id}"
        );
        for bad in [
            "tooshort",
            "01jxsnap0000000000000000000", // 27 chars
            "01JXSNAP000000000000000000",  // uppercase
            "01jxiiiiiiiiiiiiiiiiiiiiii",  // `i` not in Crockford
            "80000000000000000000000000",  // first char exceeds ULID range
            "n-0001",                      // wrong shape entirely
        ] {
            assert!(RunId::parse_str(bad).is_err(), "expected reject: {bad:?}");
        }
        for bad in TRAVERSAL_VECTORS {
            assert!(
                RunId::parse_str(bad).is_err(),
                "traversal not rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn node_id_accepts_canonical_and_rejects_malformed() {
        for ok in ["n-0001", "n-0010", "n-123456"] {
            assert!(NodeId::parse_str(ok).is_ok(), "expected accept: {ok}");
        }
        // Wrong prefix is its own error variant.
        assert!(matches!(
            NodeId::parse_str("d-0001"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        assert!(matches!(
            NodeId::parse_str("0001"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        for bad in [
            "n-1",    // too few digits
            "n-abcd", // non-digit body
            "n-",     // empty body
            "n-00a1", // mixed
        ] {
            assert!(
                matches!(
                    NodeId::parse_str(bad),
                    Err(IdValidationError::InvalidFormat { .. })
                ),
                "expected InvalidFormat: {bad:?}",
            );
        }
        for bad in TRAVERSAL_VECTORS {
            assert!(
                NodeId::parse_str(bad).is_err(),
                "traversal not rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn discussion_id_accepts_both_forms_and_rejects_malformed() {
        let gen = crate::new_discussion_id();
        assert!(
            DiscussionId::parse_str(&gen).is_ok(),
            "generator must validate: {gen}"
        );
        assert!(DiscussionId::parse_str("d-0123456789").is_ok()); // 10-char sha-prefix form
                                                                  // RFC 4648 base32 deterministic-id form (contains i/l/o/u and 2-7),
                                                                  // which the supervisor actually emits — must validate.
        assert!(DiscussionId::parse_str("d-ilou234567").is_ok());
        assert!(matches!(
            DiscussionId::parse_str("s-0123456789"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        for bad in [
            "d-short",                        // body < 10
            "d-0123456789012345678901234567", // body > 26
            "d-ABCDEFGHIJ",                   // uppercase not allowed
            "d-abc_def012",                   // `_` not alphanumeric
            "d-",                             // empty body
        ] {
            assert!(
                matches!(
                    DiscussionId::parse_str(bad),
                    Err(IdValidationError::InvalidFormat { .. })
                ),
                "expected InvalidFormat: {bad:?}",
            );
        }
        for bad in TRAVERSAL_VECTORS {
            assert!(
                DiscussionId::parse_str(bad).is_err(),
                "traversal not rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn proposal_id_accepts_both_forms_and_rejects_malformed() {
        let gen = crate::new_proposal_id();
        assert!(
            ProposalId::parse_str(&gen).is_ok(),
            "generator must validate: {gen}"
        );
        assert!(ProposalId::parse_str("s-0123456789").is_ok());
        // RFC 4648 base32 deterministic-id form (the supervisor's actual output).
        assert!(ProposalId::parse_str("s-uuuuuuuuuu").is_ok());
        assert!(matches!(
            ProposalId::parse_str("d-0123456789"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        for bad in ["s-short", "s-ABCDEFGHIJ", "s-abc.def012", "s-"] {
            assert!(
                matches!(
                    ProposalId::parse_str(bad),
                    Err(IdValidationError::InvalidFormat { .. })
                ),
                "expected InvalidFormat: {bad:?}",
            );
        }
        for bad in TRAVERSAL_VECTORS {
            assert!(
                ProposalId::parse_str(bad).is_err(),
                "traversal not rejected: {bad:?}"
            );
        }
    }

    #[test]
    fn deserialize_rejects_malformed_ids() {
        // The validating Deserialize impl is the on-read guard: a tampered
        // projection file whose key no longer validates must fail to parse.
        assert!(serde_json::from_str::<NodeId>("\"n-0001\"").is_ok());
        assert!(serde_json::from_str::<NodeId>("\"../../etc\"").is_err());
        assert!(serde_json::from_str::<DiscussionId>("\"d-../escape\"").is_err());
        assert!(serde_json::from_str::<ProposalId>("\"s-0123456789\"").is_ok());
    }

    #[test]
    fn serialize_round_trips_as_bare_string() {
        let nid = NodeId::parse_str("n-0042").unwrap();
        let json = serde_json::to_string(&nid).unwrap();
        assert_eq!(json, "\"n-0042\"");
        let back: NodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, nid);
        assert_eq!(nid.as_str(), "n-0042");
        assert_eq!(nid.to_string(), "n-0042");
    }

    #[test]
    fn error_exposes_kind_and_expected() {
        let err = NodeId::parse_str("n-x").unwrap_err();
        assert_eq!(err.kind(), "node");
        assert_eq!(err.expected(), "n-NNNN (n- followed by 4+ ASCII digits)");
    }
}
