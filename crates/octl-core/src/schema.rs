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

/// True iff `s` is a syntactically valid (possibly partial) prefix of a
/// [`RunId`]: non-empty, no longer than a full ULID, every character a lowercase
/// Crockford base32 digit, and a first character within ULID's `0..=7`
/// timestamp bound. Used by the CLI to resolve an unambiguous run-id prefix
/// (like `git`) — a value failing this is a malformed argument (`invalid_run_id`),
/// not a legitimate-but-unknown prefix. The first-char bound is enforced because
/// no valid `RunId` can begin outside `0..=7`, so an `8…`/`9…` prefix is
/// impossible rather than merely absent — reporting it as malformed keeps the
/// error class honest and consistent with how [`RunId::parse_str`] rejects a
/// full-length id with the same leading digit.
pub fn is_run_id_prefix(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= RunId::LEN
        && all_crockford_lower(s)
        && matches!(s.as_bytes().first(), Some(b'0'..=b'7'))
}

/// True iff every byte of `s` is an RFC 4648 base32 character, lowercase
/// (`a-z` and `2-7`). Distinct from Crockford in both directions: it *includes*
/// `i`/`l`/`o`/`u` and *excludes* `0`/`1`/`8`/`9`. This is the alphabet of the
/// 10-char deterministic-id (`x-<sha-prefix>`) form the supervisor emits — see
/// `octl_cli::supervise::reducer::base32_lower_10`.
fn all_rfc4648_base32_lower(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// True iff `body` is a canonical [`DiscussionId`] / [`ProposalId`] body: the
/// *syntactic* union of the two shapes real generators use.
///
/// 1. A 26-char lowercase Crockford base32 string — the `d-<ulid>` / `s-<ulid>`
///    shape from [`crate::new_discussion_id`] / [`crate::new_proposal_id`].
/// 2. A 10-char RFC 4648 base32 lowercase string (`a-z2-7`) — the
///    deterministic-id (`x-<sha-prefix>`) shape the supervisor emits.
///
/// This is a length+charset check, not proof a value was actually emitted by a
/// generator. Two deliberate looseness notes:
/// - The 10-char arm accepts any RFC 4648 base32 string (e.g. `zzzzzzzzzz`),
///   not only sha-derived ones — the supervisor's digest prefix is itself
///   uniform over that alphabet, so there is no charset rule that separates
///   "real" from "syntactically possible" output.
/// - The 26-char arm checks the Crockford charset only; unlike [`RunId`] it does
///   *not* enforce the first-char `0..=7` ULID-timestamp bound. Tightening
///   [`RunId`]/[`NodeId`] is out of scope for this validator (it would also
///   reject the looser ULID-shaped fixtures the suite still uses); the goal here
///   is just to reject the previously-accepted-but-impossible loose forms — any
///   body length other than 10 or 26, a 10-char body carrying `0`/`1`/`8`/`9`,
///   or a 26-char body carrying `i`/`l`/`o`/`u`.
fn is_canonical_disc_or_proposal_body(body: &str) -> bool {
    (body.len() == 26 && all_crockford_lower(body))
        || (body.len() == 10 && all_rfc4648_base32_lower(body))
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
/// `FromStr`, `Display`, `Debug`, `Ord` / `PartialOrd` (lexicographic over the
/// inner string), `Serialize` (as the bare string), and a validating
/// `Deserialize` (delegates to `parse_str`, so reading an old file with a
/// malformed id fails loudly rather than silently widening the type). Each
/// newtype supplies its own `parse_str` in a separate `impl` block.
///
/// `Ord` / `PartialOrd` are derived, so they forward to the inner `String`'s
/// ordering — i.e. plain `&str` byte comparison. For the fixed-width ULID forms
/// ([`RunId`], and the 26-char arm of `DiscussionId`/`ProposalId`) this
/// preserves the natural time ordering ULIDs encode in their lexical sort.
///
/// CAVEAT — this ordering is lexical, *not* numeric or semantic:
/// - [`NodeId`] is `n-` + a variable-width number, so once the counter grows a
///   digit the byte order diverges from the numeric order: `n-10000 < n-9999`.
///   Do not sort `NodeId`s expecting ascending node number; parse the body if
///   you need that.
/// - `DiscussionId`/`ProposalId` mix a 26-char and a 10-char form, so ordering
///   across the two forms is arbitrary, not creation order.
///
/// The trait is provided for `BTreeMap`/`BTreeSet` keys and stable sorts, not
/// as a claim of meaningful order for these two types.
macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// The validated id as a string slice. There is no mutable or
            /// owned-`String` accessor by design: the inner value can never be
            /// mutated into an unvalidated state after construction.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = IdValidationError;

            /// Parse via the newtype's own `parse_str`; lets callers use the
            /// `str::parse` / `FromStr` ecosystem (`s.parse::<RunId>()?`).
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse_str(s)
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
    /// Canonical length of a ULID in Crockford base32. Public so CLI-side prefix
    /// resolution can branch on "full id vs. prefix" without mirroring the
    /// constant (which would silently drift if the id shape ever changed).
    pub const LEN: usize = 26;

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
    const EXPECTED: &'static str = "n-NNNN (n- followed by 4-10 ASCII digits)";

    /// Parse and validate a `node_id`. Requires the `n-` prefix followed by
    /// 4 to 10 ASCII digits; rejects anything else (wrong prefix, too few or
    /// too many digits, non-digit body). The 10-digit ceiling covers the full
    /// `u32` counter range [`crate::format_node_id`] draws from while bounding
    /// the filename length (a defense against `ENAMETOOLONG` from a forged id).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("n-").ok_or(IdValidationError::WrongPrefix {
            kind: "node",
            expected: Self::EXPECTED,
        })?;
        if (4..=10).contains(&body.len()) && body.bytes().all(|b| b.is_ascii_digit()) {
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
    /// A validated discussion identifier: `d-` followed by exactly one of the
    /// two canonical bodies a generator emits — a 26-char lowercase Crockford
    /// base32 ULID (`d-<ulid>`) or a 10-char RFC 4648 base32 lowercase string
    /// (`d-<sha-prefix>`, the deterministic-id form). See
    /// [`is_canonical_disc_or_proposal_body`](crate::schema).
    DiscussionId
}

impl DiscussionId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str =
        "d- followed by a 26-char lowercase Crockford ULID or a 10-char RFC 4648 base32 lowercase id (a-z2-7)";

    /// Parse and validate a `discussion_id`. Requires the `d-` prefix followed
    /// by exactly one canonical body — see [`is_canonical_disc_or_proposal_body`](crate::schema).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("d-").ok_or(IdValidationError::WrongPrefix {
            kind: "discussion",
            expected: Self::EXPECTED,
        })?;
        if is_canonical_disc_or_proposal_body(body) {
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
    /// A validated spin-off proposal identifier: `s-` followed by exactly one
    /// of the two canonical bodies a generator emits — a 26-char lowercase
    /// Crockford base32 ULID (`s-<ulid>`) or a 10-char RFC 4648 base32 lowercase
    /// string (`s-<sha-prefix>`, the deterministic-id form). See
    /// [`is_canonical_disc_or_proposal_body`](crate::schema).
    ProposalId
}

impl ProposalId {
    /// Accepted-shape hint shared by every rejection.
    const EXPECTED: &'static str =
        "s- followed by a 26-char lowercase Crockford ULID or a 10-char RFC 4648 base32 lowercase id (a-z2-7)";

    /// Parse and validate a `proposal_id`. Requires the `s-` prefix followed
    /// by exactly one canonical body — see [`is_canonical_disc_or_proposal_body`](crate::schema).
    pub fn parse_str(s: &str) -> Result<Self, IdValidationError> {
        let body = s.strip_prefix("s-").ok_or(IdValidationError::WrongPrefix {
            kind: "spinoff",
            expected: Self::EXPECTED,
        })?;
        if is_canonical_disc_or_proposal_body(body) {
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
    /// Top-level DAG driver run (`/orchestrate`). Coordinates `Kind::Orchestrated`
    /// child workers. Has no worktree of its own — the orchestrator agent runs
    /// in the user's main conversation and uses the run dir only to host the
    /// event log, manifest, and final hierarchical report.
    Orchestrate,
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
            Kind::Orchestrate => "orchestrate",
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
        Kind::Orchestrate.wire_name(),
    ];

    /// Default lifecycle for a kind (design.md §7.4). `code` is
    /// interactive (human-driven inside tmux); every other MVP kind is
    /// autonomous (agent runs to completion, watchdog adjudicates).
    pub fn lifecycle(self) -> Lifecycle {
        match self {
            // `Code` is human-driven inside tmux. `Orchestrate` is also
            // interactive in the sense that the orchestrator agent runs in
            // the user's main conversation — there is no detached worker
            // for the watchdog to adjudicate, only the children it spawns.
            Kind::Code | Kind::Orchestrate => Lifecycle::Interactive,
            Kind::Spinoff
            | Kind::Orchestrated
            | Kind::Research
            | Kind::TechnicalDecision
            | Kind::MakeSkill
            | Kind::FanOut
            | Kind::Bugfix => Lifecycle::Autonomous,
        }
    }

    /// Whether this kind is a **top-level, single-node, autonomous worker** —
    /// one detached agent that materializes its own worktree and self-merges,
    /// with no children and no parent DAG driving it. These are exactly the
    /// kinds eligible for the supervisor's bounded auto-retry on an empty-handed
    /// `agent-died` (issue `autoretry-agent-died-worker`).
    ///
    /// Excludes:
    /// - `Code` / `Orchestrate` (interactive — a human drives; never force-retry);
    /// - `FanOut` (a multi-unit driver — its driver node has no agent of its own);
    /// - `Orchestrated` (a DAG child — its PARENT supervisor owns its retry policy,
    ///   so retrying it independently would desync the parent's child bookkeeping).
    ///
    /// The exhaustive `match` fails to compile when a new `Kind` is added, forcing
    /// a deliberate eligibility decision rather than a silent default.
    #[must_use]
    pub fn is_autonomous_single_node_worker(self) -> bool {
        match self {
            Kind::Spinoff
            | Kind::Research
            | Kind::TechnicalDecision
            | Kind::MakeSkill
            | Kind::Bugfix => true,
            Kind::Code | Kind::Orchestrate | Kind::FanOut | Kind::Orchestrated => false,
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
    /// Watermark: the highest event `seq` whose projection fold is durably
    /// committed. Events in `events.jsonl` with `seq > applied_seq` are
    /// *unapplied tail* events — replayed into the projections on the next
    /// lock acquisition before any new append (see
    /// [`crate::events::append_and_apply_event`]). This is what makes
    /// append-then-apply atomic across a reducer crash: the event log can run
    /// ahead of the projections, but the gap is always healed before the next
    /// writer observes stale state.
    ///
    /// `#[serde(default)]` so a legacy `manifest.json` written before this
    /// field existed deserializes with `applied_seq = 0`. Such a manifest
    /// self-migrates on its next write: the catch-up replay re-folds the whole
    /// log — every event a no-op, because legacy state was already projected
    /// synchronously under the old append-then-apply path — and advances the
    /// watermark to `last_seq`. No separate migration pass or schema bump is
    /// required (the field is purely additive to a derived-cache file).
    #[serde(default)]
    pub applied_seq: u64,
    /// Unique run identifier (ULID). Validated on read.
    pub run_id: RunId,
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
    /// tmux session orchestratectl created to host this run's headless windows
    /// (`--headless` / `--tmux-session <name>`), if any. `None` for a foreground
    /// run whose window lives in the user's own session — that session is never
    /// a teardown target. When set, the supervisor kills this session once its
    /// last orchestratectl-owned window is torn down and only the synthetic
    /// bootstrap shell window remains, so an empty headless session is not left
    /// behind (issue `headless-tmux-session-not-torn-down`). `#[serde(default)]`
    /// keeps a manifest written before this field existed readable.
    #[serde(default)]
    pub managed_tmux_session: Option<String>,
    /// Completion-notification command registered at `run create --notify`,
    /// if any. When the run reaches a terminal state (`done | failed |
    /// cancelled`) the supervisor runs this command (at-least-once, deduped on a
    /// durable `run.notified` marker event — the healthy path fires once, a
    /// crash between firing and recording may re-fire) with `OCTL_RUN_ID` /
    /// `OCTL_STATUS` / `OCTL_SUMMARY` (and `OCTL_RUN_KIND` / `OCTL_RUN_TITLE`)
    /// in its environment, BEFORE teardown removes the worktree/window. This is
    /// how a spawning session learns of completion without polling (issue
    /// `no-completion-notification-to-parent`). `None` for a run created without
    /// `--notify`; `#[serde(default)]` keeps a manifest written before this
    /// field existed readable.
    #[serde(default)]
    pub notify_cmd: Option<String>,
    /// The agent runtime selected for this run's worker
    /// (`claude` | `pi`), resolved at `run create`
    /// via the flag > env > config > default precedence and recorded here as
    /// provenance. This is the *selected* harness — recorded before the worker is
    /// spawned, so it reflects intent even if the spawn later fails. `None` for a
    /// manifest written before this field existed
    /// (`#[serde(default)]`) — such legacy runs predate harness selection and
    /// were all `claude`. Surfaced on `run show` / `run list --json`.
    #[serde(default)]
    pub harness: Option<String>,
    /// Number of nodes created in this run (denormalized counter).
    pub node_count: u32,
    /// Count of currently open discussions (denormalized counter).
    pub open_discussions: u32,
    /// Count of currently pending spin-off proposals (denormalized counter).
    pub pending_spinoffs: u32,
    /// Run that spawned this run, if it is itself a child.
    pub parent_run_id: Option<RunId>,
    /// Node in the parent run that spawned this run, if any.
    pub parent_node_id: Option<NodeId>,
}

/// `(child_run_id, child_node_id)` pointer recorded in `Node::children`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildRef {
    /// Run id of the spawned child. Validated on read.
    pub run_id: RunId,
    /// Node id within the child run. Validated on read.
    pub node_id: NodeId,
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
    /// Run this node belongs to. Validated on read.
    pub run_id: RunId,
    /// Parent node within the same run, if this is a sub-node.
    pub parent_node_id: Option<NodeId>,
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
    /// The commit SHA the node's branch/worktree was forked from at spawn
    /// (the branch tip the moment `create.sh` materialized the worktree). It
    /// is the fixed reference point that lets the supervisor tell "this branch
    /// produced work that merged into source" from "this branch never diverged
    /// from its fork point": a branch still at `base_sha` is trivially an
    /// ancestor of its source branch but has merged nothing, so it must NOT be
    /// reconciled to success or torn down (that would drop a live agent's
    /// uncommitted work). Only a branch whose tip has moved past `base_sha`
    /// *and* is now an ancestor of the run's `source_branch` is a confirmed
    /// merge (issues `false-failed-after-merge` /
    /// `supervisor-stuck-pending-after-self-merge`). `#[serde(default)]` keeps a
    /// node written before this field existed readable (`None` → the
    /// git-reconcile fallback simply does not fire for it).
    #[serde(default)]
    pub base_sha: Option<String>,
    /// tmux window hosting the node's agent, if interactive. This is the
    /// human-readable window *name* — not unique across sessions and blind to
    /// non-default sockets. Kept for display and as the legacy liveness key;
    /// prefer [`Node::tmux_identity`] when present.
    pub tmux_window: Option<String>,
    /// Fully-qualified tmux identity (`session:window_id` + socket path)
    /// captured at spawn time. `None` for nodes registered before create.sh
    /// emitted the qualified fields — those fall back to bare-name matching on
    /// [`Node::tmux_window`]. New spawns always populate this when create.sh
    /// returns it.
    #[serde(default)]
    pub tmux_identity: Option<TmuxIdentity>,
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
    /// Number of times the supervisor has auto-retried this node after an
    /// empty-handed `agent-died` (issue `autoretry-agent-died-worker`). The
    /// DURABLE, restart-safe bound on the bounded-retry loop: each `node.retry`
    /// event increments it, and the watchdog terminalizes the run `failed` once
    /// it reaches `RETRY_MAX_ATTEMPTS`. `#[serde(default)]` keeps a node written
    /// before this field existed readable (`0` — never retried).
    #[serde(default)]
    pub retry_attempts: u32,
}

/// A fully-qualified tmux window identity recorded at spawn time.
///
/// `tmux_window` (the human name) is not unique across sessions, and a bare
/// `tmux list-windows -a` cannot see windows on a non-default socket. This
/// triple pins the exact window the agent runs in — `session:window_id` is
/// unique per server, `window_id` (the `@NNNN` form) survives renames, and
/// `socket` disambiguates multiple tmux servers. The watchdog matches on this
/// when present (design.md §8.1).
///
/// `pane_id` (the `%NN` form) pins the agent's *specific* pane within that
/// window, recorded at spawn. Window-owning operations (`kill-window` teardown —
/// the supervisor owns the whole window per the cleanup invariants) key off
/// `window_id`; only per-pane operations that must not follow the window's
/// *active* pane — chiefly `pipe-pane` agent-log capture — use `pane_id`. It is
/// `None` for a run spawned before create.sh emitted the field; capture then
/// falls back to `window_id` (issue `capture-agent-pane-by-pane-id`).
///
/// The watchdog's liveness probe still keys off `window_id` (correct for the
/// single-pane autonomous path). A pane-aware liveness probe — needed so a split
/// interactive window whose agent pane dies while a user shell pane survives is
/// still seen as dead — is a follow-up (`watchdog-pane-aware-liveness`), not this
/// change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxIdentity {
    /// Server socket path (`#{socket_path}`). `None` if create.sh could not
    /// read it; the watchdog then queries tmux on its default socket.
    #[serde(default)]
    pub socket: Option<String>,
    /// Session that owns the window (`#{session_name}`).
    pub session: String,
    /// Stable window id in `@NNNN` form (`#{window_id}`). Survives renames and
    /// is unique within the server.
    pub window_id: String,
    /// Stable pane id in `%NN` form (`#{pane_id}`), recorded at spawn — the
    /// agent's own pane. `None` for a run whose create.sh predates the field
    /// (back-compat: old state deserializes with `pane_id: None`). Prefer
    /// [`TmuxIdentity::capture_target`] over reading this directly.
    #[serde(default)]
    pub pane_id: Option<String>,
}

impl TmuxIdentity {
    /// The tmux target for a per-pane operation that must hit the agent's own
    /// pane, not the window's *active* pane: the recorded `pane_id` when
    /// present, else the `window_id` (which resolves to the active pane).
    ///
    /// Used by agent-log capture (`pipe-pane`). Window-level operations
    /// (`kill-window`, liveness) must NOT use this — they key off `window_id`
    /// directly so they act on the whole window.
    ///
    /// A recorded `pane_id` is preferred only when non-empty; an empty string
    /// (a directly-deserialized/corrupt state that the reducer/spawn normalizers
    /// never produce) is treated as absent so capture never targets `-t ""`.
    pub fn capture_target(&self) -> &str {
        self.pane_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&self.window_id)
    }
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
    /// Run this discussion belongs to. Validated on read.
    pub run_id: RunId,
    /// Node that opened the discussion. Validated on read.
    pub node_id: NodeId,
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
    /// Run this proposal belongs to. Validated on read.
    pub run_id: RunId,
    /// Node that proposed the spin-off. Validated on read.
    pub node_id: NodeId,
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
///
/// `run_id` / `node_id` are the typed id newtypes, so deserializing an
/// `events.jsonl` line validates the whole envelope on read: a malformed
/// `run_id` or `node_id` fails the `serde` parse at the read boundary (the
/// id newtypes' validating `Deserialize`) rather than being carried as an
/// unvalidated `String` until some later path helper. The parse failure
/// surfaces as whatever error the reader maps a bad line to — e.g. a
/// newline-terminated bad line is [`Error::CorruptEventLog`] from both
/// [`read_all_events`] and [`find_prior_with_key`], which share one physical
/// reader and torn-tail policy. The reducer still performs its own per-event
/// checks (envelope `run_id` matches the run it is folded into; `data`-borne
/// ids parse), but the envelope ids can no longer be the unvalidated party.
///
/// [`read_all_events`]: crate::events::read_all_events
/// [`find_prior_with_key`]: crate::events
/// [`Error::CorruptEventLog`]: crate::Error::CorruptEventLog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Wall-clock timestamp the event was appended.
    pub ts: DateTime<Utc>,
    /// Monotonic per-run sequence number (recovered on append).
    pub seq: u64,
    /// Event kind discriminator (e.g. `node.created`, `discussion.opened`).
    pub kind: String,
    /// Run the event belongs to. Validated on read.
    pub run_id: RunId,
    /// Node the event concerns, when applicable. Validated on read.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub node_id: Option<NodeId>,
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

    /// The bounded auto-retry eligibility gate (issue `autoretry-agent-died-worker`)
    /// must include exactly the autonomous single-node worker kinds and exclude
    /// interactive kinds, the fan-out driver, and the DAG child.
    #[test]
    fn autonomous_single_node_worker_set_is_exact() {
        for k in [
            Kind::Spinoff,
            Kind::Research,
            Kind::TechnicalDecision,
            Kind::MakeSkill,
            Kind::Bugfix,
        ] {
            assert!(
                k.is_autonomous_single_node_worker(),
                "{k:?} should be retry-eligible"
            );
            assert_eq!(k.lifecycle(), Lifecycle::Autonomous);
        }
        for k in [
            Kind::Code,         // interactive — a human drives
            Kind::Orchestrate,  // interactive driver
            Kind::FanOut,       // multi-unit driver
            Kind::Orchestrated, // DAG child — parent owns its retry policy
        ] {
            assert!(
                !k.is_autonomous_single_node_worker(),
                "{k:?} must NOT be retry-eligible"
            );
        }
    }

    /// Back-compat acceptance criterion (issue `capture-agent-pane-by-pane-id`):
    /// a `TmuxIdentity` persisted before `pane_id` existed — with the field
    /// entirely absent, or written as an explicit `null` — must still
    /// deserialize, yielding `pane_id: None` and a `window_id` capture target.
    #[test]
    fn tmux_identity_deserializes_legacy_state_without_pane_id() {
        // Field entirely absent (a state file written by an older binary).
        let absent: TmuxIdentity = serde_json::from_value(serde_json::json!({
            "socket": null,
            "session": "octl",
            "window_id": "@42",
        }))
        .expect("legacy identity without pane_id must deserialize");
        assert_eq!(absent.pane_id, None);
        assert_eq!(absent.capture_target(), "@42");

        // Field present but explicitly null.
        let null: TmuxIdentity = serde_json::from_value(serde_json::json!({
            "socket": null,
            "session": "octl",
            "window_id": "@42",
            "pane_id": null,
        }))
        .expect("identity with explicit null pane_id must deserialize");
        assert_eq!(null.pane_id, None);
        assert_eq!(null.capture_target(), "@42");
    }

    /// `capture_target` prefers a recorded `pane_id` (`%NN`) over the window id,
    /// but treats an empty `pane_id` as absent (never targets `-t ""`).
    #[test]
    fn capture_target_prefers_nonempty_pane_id() {
        let with_pane = TmuxIdentity {
            socket: None,
            session: "octl".into(),
            window_id: "@42".into(),
            pane_id: Some("%7".into()),
        };
        assert_eq!(with_pane.capture_target(), "%7");

        let empty_pane = TmuxIdentity {
            pane_id: Some(String::new()),
            ..with_pane.clone()
        };
        assert_eq!(empty_pane.capture_target(), "@42");
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
            "n-1",           // too few digits
            "n-abcd",        // non-digit body
            "n-",            // empty body
            "n-00a1",        // mixed
            "n-00000000000", // 11 digits — over the 10-digit ceiling
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
    fn discussion_id_accepts_both_canonical_forms_and_rejects_malformed() {
        let gen = crate::new_discussion_id();
        assert!(
            DiscussionId::parse_str(&gen).is_ok(),
            "generator must validate: {gen}"
        );
        // 26-char lowercase Crockford ULID form.
        assert!(DiscussionId::parse_str("d-01arz3ndektsv4rrffq69g5fav").is_ok());
        // 10-char RFC 4648 base32 deterministic-id form (contains i/l/o/u and
        // 2-7), which the supervisor actually emits — must validate.
        assert!(DiscussionId::parse_str("d-ilou234567").is_ok());
        assert!(DiscussionId::parse_str("d-abcdefghij").is_ok());
        // A 10-char body of all-`z` is a legitimate RFC 4648 base32 string
        // (`z` is in `a-z2-7`), so it is accepted — same class as `ilou234567`.
        // The issue's success-criteria example listing `d-zzzzzzzzzz` among
        // "now-invalid" forms is imprecise: it IS canonical RFC 4648 base32 and
        // there is no charset-based rule that rejects it without also rejecting
        // the supervisor's real deterministic ids.
        assert!(DiscussionId::parse_str("d-zzzzzzzzzz").is_ok());
        assert!(matches!(
            DiscussionId::parse_str("s-0123456789"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        for bad in [
            "d-0123456789",                   // 10 chars but `0`/`1` ∉ RFC 4648 base32
            "d-short",                        // body < 10
            "d-abcdefghijk",                  // 11 chars — between the two forms
            "d-01arz3ndektsv4rrffq69g5fa",    // 25 chars — one short of a ULID
            "d-0123456789012345678901234567", // body > 26
            "d-01arz3ndeilov4rrffq69g5fav",   // 26 chars but carries `i`/`l`/`o` ∉ Crockford
            "d-ABCDEFGHIJ",                   // uppercase not allowed
            "d-abc_def012",                   // `_` outside both alphabets
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
    fn proposal_id_accepts_both_canonical_forms_and_rejects_malformed() {
        let gen = crate::new_proposal_id();
        assert!(
            ProposalId::parse_str(&gen).is_ok(),
            "generator must validate: {gen}"
        );
        // 26-char lowercase Crockford ULID form.
        assert!(ProposalId::parse_str("s-01arz3ndektsv4rrffq69g5fav").is_ok());
        // 10-char RFC 4648 base32 deterministic-id form (the supervisor's
        // actual output); `u` is valid RFC 4648 base32.
        assert!(ProposalId::parse_str("s-uuuuuuuuuu").is_ok());
        assert!(matches!(
            ProposalId::parse_str("d-0123456789"),
            Err(IdValidationError::WrongPrefix { .. })
        ));
        for bad in [
            "s-0123456789",                 // 10 chars but `0`/`1` ∉ RFC 4648 base32
            "s-short",                      // body < 10
            "s-abcdefghijk",                // 11 chars — between the two forms
            "s-01arz3ndeilov4rrffq69g5fav", // 26 chars but carries `i`/`l`/`o` ∉ Crockford
            "s-ABCDEFGHIJ",                 // uppercase not allowed
            "s-abc.def012",                 // `.` outside both alphabets
            "s-",                           // empty body
        ] {
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
        // Canonical 10-char RFC 4648 base32 body deserializes; the old
        // loose-charset `s-0123456789` (carries `0`/`1`) no longer does.
        assert!(serde_json::from_str::<ProposalId>("\"s-abcdefghij\"").is_ok());
        assert!(serde_json::from_str::<ProposalId>("\"s-0123456789\"").is_err());
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
        assert_eq!(err.expected(), "n-NNNN (n- followed by 4-10 ASCII digits)");
    }

    #[test]
    fn event_deserialize_validates_envelope_ids() {
        // The whole `events.jsonl` envelope is now validated on read: the
        // typed `run_id` / `node_id` fields parse through the id newtypes, so
        // a malformed envelope id fails the deserialize rather than being
        // carried downstream as an unchecked string.
        let ok = r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"node.created","run_id":"01jxsnap000000000000000000","node_id":"n-0001","data":{}}"#;
        assert!(serde_json::from_str::<Event>(ok).is_ok());

        // Invalid `run_id` (not a 26-char ULID) fails the parse.
        let bad_run = r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"run.status","run_id":"not-a-ulid","data":{}}"#;
        assert!(serde_json::from_str::<Event>(bad_run).is_err());

        // Invalid top-level `node_id` (too few digits) also fails the parse.
        let bad_node = r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"node.status","run_id":"01jxsnap000000000000000000","node_id":"n-1","data":{}}"#;
        assert!(serde_json::from_str::<Event>(bad_node).is_err());
    }

    #[test]
    fn from_str_and_ord_delegate_to_inner() {
        use std::str::FromStr;
        // `FromStr` mirrors `parse_str`, so the `str::parse` ecosystem works.
        assert!(RunId::from_str("01jxsnap000000000000000000").is_ok());
        assert!("n-0001".parse::<NodeId>().is_ok());
        assert!("n-x".parse::<NodeId>().is_err());

        // `Ord` is lexicographic over the inner string; for ULIDs that is the
        // natural time-encoded order.
        let a = RunId::parse_str("01jxsnap000000000000000000").unwrap();
        let b = RunId::parse_str("02jxsnap000000000000000000").unwrap();
        assert!(a < b);
        let mut v = vec![b.clone(), a.clone()];
        v.sort();
        assert_eq!(v, vec![a, b]);
    }
}
