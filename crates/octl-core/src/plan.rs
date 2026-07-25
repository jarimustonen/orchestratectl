//! `plan.json` v2 — serde types + structural validator (design.md §4, §7, §13).
//!
//! `plan.json` is the **interface contract** the spec-node writes and the
//! supervisor + orchestrator read. It is immutable per revision, versioned, and
//! provenance-bearing. This module provides:
//!
//! - The serde [`Plan`] type (and its parts) mirroring `plan-schema.md` v2.
//! - A structural [`validate_plan`] / [`parse_and_validate_plan`] pass that
//!   rejects a bad plan with a domain-typed [`PlanValidationError`] (the CLI
//!   maps these to its `schema_violation` envelope at the boundary, exactly as
//!   it does for [`crate::report::ReportValidationError`]).
//! - [`PLAN_V2_JSON_SCHEMA`], the checked-in JSON Schema artifact, so external
//!   readers/writers validate against a single source of truth. A drift-guard
//!   test keeps the Rust types and the JSON Schema in agreement.
//!
//! # Compatibility semantics (design.md §13, `plan-schema.md` "Principles")
//!
//! `schema_version` gates the file with *real* compatibility semantics — this
//! is not "ignore everything unknown":
//!
//! - Readers **reject unsupported major versions** ([`SUPPORTED_PLAN_SCHEMAS`]).
//! - Readers **reject undeclared fields** — any key not in the v2 shape is a
//!   rejection. On the map-like objects (plan, `feature`, `baseline`,
//!   `chunks[]`, `chunks[].checks[]`) this is [`PlanValidationError::UnknownField`],
//!   gated by a **per-object-shape** allowlist ([`tolerated_fields`]): a field
//!   ratified as additive on one shape is tolerated there and nowhere else. On
//!   `acceptance[]` items (a tagged enum) it is a `deny_unknown_fields`
//!   deserialization error ([`PlanValidationError::Malformed`]) — the same
//!   stance the JSON Schema takes, with no additive seam in v2. The allowlists
//!   are empty in v2, so every unknown key is currently rejected; a future minor
//!   registers an additive optional field against its shape (and in the JSON
//!   Schema) so older readers tolerate it, and only then. Schema growth
//!   otherwise goes gap-event → reviewed proposal → versioned schema.
//!
//! This module is **read-only + validation types**. It does not touch the
//! reducer, the lock layer, or any event-append path (state-integrity
//! invariants), and it is not yet wired into a live path — T3 (deterministic
//! floor) and T5 (supervisor) consume it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The current `plan.json` schema major version this crate writes.
pub const PLAN_SCHEMA_VERSION: u32 = 2;

/// All `plan.json` schema major versions this crate can read. A file whose
/// `schema_version` is not listed here is rejected outright
/// ([`PlanValidationError::UnsupportedSchemaVersion`]) — tolerant reading is
/// limited to additive optional fields *within* a supported major, never to a
/// whole unknown major.
pub const SUPPORTED_PLAN_SCHEMAS: &[u32] = &[2];

/// Field names tolerated when they appear as unknown keys in an otherwise-valid
/// plan — the governed-evolution seam (design.md §13). Empty in v2: no additive
/// optional field has been ratified yet, so every unknown key is currently a
/// rejection.
///
/// This is the flattened union across every object shape, exposed for
/// documentation and the `expected` hint. The *operative* allowlist is
/// **per-object-shape** ([`tolerated_fields`]): a field ratified as additive on
/// `chunks[]` is tolerated there and nowhere else — a field's optionality
/// depends on its location, not just its name, so a global name-only allowlist
/// would leak a `chunks[].retries` tolerance onto `feature`, `baseline`, and
/// the top level. A future minor registers a new field against its specific
/// [`ObjectShape`] (and in the JSON Schema) so older readers tolerate it there;
/// anything not listed for that shape is a possibly-required unknown and is
/// rejected.
pub const TOLERATED_OPTIONAL_FIELDS: &[&str] = &[];

/// The object shapes an unknown-field check runs against — each carries its own
/// additive-optional allowlist ([`tolerated_fields`]), so the governed-evolution
/// seam is scoped to a location rather than a bare field name. (`Acceptance`
/// items are absent: they reject unknowns at deserialize time via
/// `deny_unknown_fields`, matching the schema, and have no seam in v2.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectShape {
    /// The top-level plan object.
    Plan,
    /// `feature`.
    Feature,
    /// `baseline`.
    Baseline,
    /// A `chunks[]` element.
    Chunk,
    /// A `chunks[].checks[]` element.
    Check,
}

impl ObjectShape {
    /// Dotted path fragment naming this shape in an error (the chunk/check
    /// arms fill in the index at the call site).
    fn label(self) -> &'static str {
        match self {
            ObjectShape::Plan => "<plan>",
            ObjectShape::Feature => "feature",
            ObjectShape::Baseline => "baseline",
            ObjectShape::Chunk => "chunks[]",
            ObjectShape::Check => "checks[]",
        }
    }
}

/// The additive-optional fields tolerated on a given object shape. Empty for
/// every shape in v2 — the seam exists so a ratified field can be admitted at
/// exactly one location without widening any other (design.md §13).
const fn tolerated_fields(shape: ObjectShape) -> &'static [&'static str] {
    match shape {
        ObjectShape::Plan
        | ObjectShape::Feature
        | ObjectShape::Baseline
        | ObjectShape::Chunk
        | ObjectShape::Check => &[],
    }
}

/// The checked-in JSON Schema (Draft 2020-12) describing `plan.json` v2.
///
/// This is the machine-readable artifact external readers/writers validate
/// against. The operative source of truth for the supervisor/spec-node is the
/// [`Plan`] type + [`validate_plan`] in this module; a drift-guard test
/// (`json_schema_matches_rust_types`) asserts the two never diverge on the
/// version constant, the required top-level fields, the [`Tier`] enum, and the
/// acceptance `kind` discriminants.
pub const PLAN_V2_JSON_SCHEMA: &str = include_str!("../schemas/plan.v2.schema.json");

/// Return the checked-in JSON Schema source for `plan.json` v2.
#[must_use]
pub fn plan_v2_json_schema() -> &'static str {
    PLAN_V2_JSON_SCHEMA
}

/// The checked-in canonical `plan.json` v2 example (`plan-schema.md` sample),
/// exposed so a spec-node prompt can show the model the exact target shape.
pub const PLAN_V2_EXAMPLE: &str = include_str!("../schemas/plan.v2.example.json");

/// Return the canonical `plan.json` v2 example document.
#[must_use]
pub fn plan_v2_json_schema_example() -> &'static str {
    PLAN_V2_EXAMPLE
}

/// A `plan.json` v2 document (design.md §4, §7; `plan-schema.md`).
///
/// Deserialization is deliberately *tolerant* of unknown keys (they are
/// captured into `extra` rather than failing the parse) so the structural
/// validator can decide their fate per the compatibility semantics above —
/// rejecting undeclared fields while leaving room for an allowlisted additive
/// optional field. Always construct through [`parse_and_validate_plan`] (or run
/// [`validate_plan`] after deserializing) before trusting a `Plan`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    /// Major schema version. Readers reject unsupported majors.
    pub schema_version: u32,
    /// Immutable revision of this plan; chunk attempts reference it.
    pub plan_rev: u32,
    /// The `intent.md` revision this plan targets (intent is referenced, not
    /// embedded).
    pub intent_rev: u32,
    /// Feature identity: slug + source/integration branches.
    pub feature: Feature,
    /// Snapshot at `feat/<slug>` fork; the floor + verify diff against it.
    pub baseline: Baseline,
    /// Whole-feature intent gate; each item is a `check` or an `assertion`,
    /// and at least one must be a `check`.
    pub acceptance: Vec<Acceptance>,
    /// The DAG of implementation chunks (`deps` form an acyclic graph).
    pub chunks: Vec<Chunk>,
    /// Unrecognized top-level keys, captured for the compatibility check rather
    /// than silently dropped. Serialized back out verbatim so a tolerated
    /// additive field round-trips.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Feature identity block (owner: orchestrator/spec).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Feature {
    /// Feature slug (e.g. `user-csv-export`).
    pub slug: String,
    /// Branch the feature forks from (e.g. `main`).
    pub source_branch: String,
    /// Integration branch the chunks merge into (e.g. `feat/user-csv-export`).
    pub integration_branch: String,
    /// Unrecognized keys, captured for the compatibility check.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Baseline snapshot captured at the `feat/<slug>` fork (owner: supervisor).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Baseline {
    /// Git ref the snapshot was taken at (e.g. `feat/<slug>@fork`).
    pub r#ref: String,
    /// Hash of the passing-test list at baseline (floor: no baseline pass may
    /// regress).
    pub test_passlist_hash: String,
    /// Hash of the clippy-warning list at baseline (floor: no new warnings).
    pub clippy_warnings_hash: String,
    /// Unrecognized keys, captured for the compatibility check.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A whole-feature acceptance criterion — an executable `check` or an
/// LLM-judged `assertion`. Internally tagged on `kind`, so an unknown `kind`
/// fails deserialization (surfaced as [`PlanValidationError::Malformed`]).
///
/// `deny_unknown_fields` makes an undeclared key inside a variant (e.g. a
/// `run` on an `assertion`, or a stray `budget` on a `check`) a hard
/// deserialization error, matching the JSON Schema's `additionalProperties:
/// false` on each acceptance variant. Acceptance items therefore have **no
/// additive-optional seam** in v2 — the same stance the schema takes; a future
/// minor that needs one would move to a captured-`extra` shape (as the
/// [`Chunk`]/[`Check`] structs use) under governed evolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Acceptance {
    /// An executable end-to-end check (`desc` + shell/test `run`, with optional
    /// `cwd` / `expect_exit` precision — same flexible shape as [`Check`]).
    Check {
        /// The general goal of the check — what it verifies.
        desc: String,
        /// A flexible shell command the supervisor executes.
        run: String,
        /// Optional working directory (repo-relative) to run `run` in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Optional expected exit code (absent = exit 0).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect_exit: Option<i32>,
    },
    /// An LLM-judged criterion (no executable command).
    Assertion {
        /// Human-readable description of the asserted property.
        desc: String,
    },
}

impl Acceptance {
    /// True for the executable [`Acceptance::Check`] arm.
    #[must_use]
    pub fn is_check(&self) -> bool {
        matches!(self, Acceptance::Check { .. })
    }
}

/// One implementation chunk (owner: spec).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chunk {
    /// Unique id within the plan; referenced by other chunks' `deps`.
    pub id: String,
    /// Short human-readable title.
    pub title: String,
    /// Ids of chunks this one depends on (the DAG edges).
    #[serde(default)]
    pub deps: Vec<String>,
    /// Starting model-tier hint; the orchestrator owns promotion.
    pub tier: Tier,
    /// Turnkey, self-contained implementation brief.
    pub brief: String,
    /// Repo-relative files this chunk may touch — a merge-time constraint, not
    /// just a hint.
    pub files_touched: Vec<String>,
    /// Executable per-chunk checks (`desc` + `run`); at least one required.
    pub checks: Vec<Check>,
    /// LLM-judged criteria, additive above the deterministic floor.
    #[serde(default)]
    pub assertions: Vec<String>,
    /// If true, the supervisor blocks a merge that added/modified no tests.
    #[serde(default)]
    pub requires_tests: bool,
    /// Unrecognized keys, captured for the compatibility check.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An executable check: the general goal plus a flexible runnable form that
/// proves it. The goal (`desc`) is always communicated and the command (`run`)
/// is a free-form shell string; precision (`cwd`, `expect_exit`) is available
/// but not forced (owner decision 2026-07-23, `plan-check-run-contract`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    /// The general goal of the check — what it verifies. Always present,
    /// human- and LLM-readable.
    pub desc: String,
    /// A flexible shell command the supervisor executes (via `sh -c`).
    pub run: String,
    /// Optional working directory (repo-relative) to run `run` in; when absent
    /// the check runs at the worktree root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Optional expected exit code — the check passes iff the command exits with
    /// this code. Absent means the default: exit 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_exit: Option<i32>,
    /// Unrecognized keys, captured for the compatibility check.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Model-tier hint for a chunk. Serialized as its lowercase wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Cheapest tier — turnkey briefs, no architectural reasoning.
    Code,
    /// Mid tier.
    Mid,
    /// Highest tier — reserved for the hardest chunks / promotions.
    High,
}

impl Tier {
    /// The lowercase wire name serde (de)serializes this tier as.
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Tier::Code => "code",
            Tier::Mid => "mid",
            Tier::High => "high",
        }
    }

    /// Every tier's wire name, in declaration order — the single source of
    /// truth for "the set of accepted tiers" (mirrors [`crate::schema::Kind`]).
    pub const WIRE_NAMES: &'static [&'static str] = &[
        Tier::Code.wire_name(),
        Tier::Mid.wire_name(),
        Tier::High.wire_name(),
    ];
}

/// A `plan.json` document failed schema validation.
///
/// Every variant names one violation. The CLI renders these as a
/// `schema_violation` error; [`PlanValidationError::expected`] supplies the
/// machine-readable `expected` hint for the variants that carry one, mirroring
/// [`crate::report::ReportValidationError`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanValidationError {
    /// The document root was not a JSON object.
    #[error("plan must be a JSON object")]
    NotObject,

    /// The required `schema_version` field was absent.
    #[error("plan missing required field `schema_version`")]
    SchemaVersionMissing,

    /// `schema_version` was present but not a non-negative integer.
    #[error("field `schema_version` must be a non-negative integer")]
    SchemaVersionNotInt,

    /// `schema_version` declared a major this build does not support.
    #[error("unsupported plan schema_version {found} (supported: {supported:?})")]
    UnsupportedSchemaVersion {
        /// The `schema_version` value read from the document.
        found: u64,
        /// The majors this build accepts (see [`SUPPORTED_PLAN_SCHEMAS`]).
        supported: Vec<u32>,
    },

    /// The document is a supported version but does not match the v2 shape
    /// (missing required field, wrong type, unknown acceptance `kind`, unknown
    /// `tier`, …). Carries the underlying serde message.
    #[error("plan is malformed: {message}")]
    Malformed {
        /// The serde deserialization message.
        message: String,
    },

    /// An undeclared field appeared and is not in [`TOLERATED_OPTIONAL_FIELDS`].
    #[error("unknown field `{field}` at {path} (not a tolerated additive optional field)")]
    UnknownField {
        /// Dotted path to the object carrying the unknown key.
        path: String,
        /// The offending field name.
        field: String,
    },

    /// A required string field was empty (or whitespace-only).
    #[error("field `{path}` must be a non-empty string")]
    EmptyString {
        /// Dotted path to the offending field.
        path: String,
    },

    /// `acceptance[]` was empty.
    #[error("`acceptance` must contain at least one item")]
    AcceptanceEmpty,

    /// `acceptance[]` contained no executable `check` (only assertions).
    #[error("`acceptance` must contain at least one executable check (not all assertions)")]
    AcceptanceNoCheck,

    /// `chunks[]` was empty.
    #[error("`chunks` must contain at least one chunk")]
    ChunksEmpty,

    /// A chunk id was empty or used characters outside `[A-Za-z0-9_.-]` (with a
    /// leading alphanumeric). Chunk ids must be safe to reference and log.
    #[error("chunk id {id:?} is invalid: expected {expected}")]
    InvalidChunkId {
        /// The offending id.
        id: String,
        /// Accepted-shape hint.
        expected: &'static str,
    },

    /// Two chunks shared an id.
    #[error("duplicate chunk id {id:?}")]
    DuplicateChunkId {
        /// The repeated id.
        id: String,
    },

    /// A chunk's `deps` referenced an id that no chunk defines.
    #[error("chunk {chunk:?} depends on unknown chunk {dep:?}")]
    UnknownDep {
        /// The depending chunk.
        chunk: String,
        /// The dangling dependency id.
        dep: String,
    },

    /// A chunk listed the same dependency more than once.
    #[error("chunk {chunk:?} lists duplicate dependency {dep:?}")]
    DuplicateDep {
        /// The depending chunk.
        chunk: String,
        /// The repeated dependency id.
        dep: String,
    },

    /// The dependency graph contained a cycle.
    #[error("chunk dependency graph has a cycle: {}", cycle.join(" -> "))]
    DependencyCycle {
        /// The chunk ids forming the cycle, in order, with the entry id
        /// repeated at the end (e.g. `["c1", "c2", "c1"]`).
        cycle: Vec<String>,
    },

    /// A chunk declared no executable `check`.
    #[error("chunk {chunk:?} must have at least one check")]
    ChunkNoCheck {
        /// The offending chunk id.
        chunk: String,
    },

    /// A chunk declared no `files_touched` entries.
    #[error("chunk {chunk:?} must declare at least one file in `files_touched`")]
    ChunkNoFiles {
        /// The offending chunk id.
        chunk: String,
    },

    /// A `files_touched` entry was not a safe repo-relative path.
    #[error("path {path:?} in chunk {chunk:?} is not a safe repo-relative path (no absolute paths, `~`, `\\`, `:`, control chars, or `.`/`..`/empty components)")]
    UnsafePath {
        /// The offending chunk id.
        chunk: String,
        /// The offending path.
        path: String,
    },

    /// A check's optional `cwd` was not a safe repo-relative directory. Held to
    /// the same lexical guard as `files_touched` ([`is_safe_repo_relative`]) —
    /// `cwd` controls *where a shell command executes*, so an absolute path
    /// (`/etc`) or a `..`/`~` traversal would let a check escape the worktree the
    /// floor gates. Absence already means "the worktree root", so a bare `.` is
    /// rejected too — there is one spelling for root, not two.
    #[error("cwd {path:?} at {location} is not a safe repo-relative directory (no absolute paths, `~`, `\\`, `:`, control chars, or `.`/`..`/empty components; omit `cwd` for the worktree root)")]
    UnsafeCwd {
        /// Dotted path to the offending `cwd` (e.g. `chunks[c1].checks[0].cwd`).
        location: String,
        /// The offending path.
        path: String,
    },

    /// A check's optional `expect_exit` was outside the range a `sh -c` process
    /// can actually report. A shell exit status is `0..=255`; a value outside it
    /// (negative, or `> 255`) could never match `code()` and would make the check
    /// permanently un-passable, so it is rejected at validation rather than
    /// silently failing every run.
    #[error("expect_exit {value} at {location} is out of range (a shell exit status is 0..=255)")]
    ExpectExitOutOfRange {
        /// Dotted path to the offending `expect_exit`.
        location: String,
        /// The offending value.
        value: i64,
    },
}

impl PlanValidationError {
    /// The machine-readable `expected` hint for this error, if any — mirrors
    /// [`crate::report::ReportValidationError::expected`] so the CLI can attach
    /// the same structured payload.
    #[must_use]
    pub fn expected(&self) -> Option<Value> {
        match self {
            Self::SchemaVersionMissing | Self::SchemaVersionNotInt => {
                Some(serde_json::json!({"field": "schema_version", "type": "integer"}))
            }
            Self::UnsupportedSchemaVersion { supported, .. } => {
                Some(serde_json::json!({"field": "schema_version", "supported": supported}))
            }
            Self::UnknownField { .. } => {
                Some(serde_json::json!({"tolerated_optional": TOLERATED_OPTIONAL_FIELDS}))
            }
            _ => None,
        }
    }
}

/// Parse a raw JSON value as a `plan.json` v2 document and validate it.
///
/// The two-phase entry point the supervisor/spec-node use. It gates the version
/// *before* deserializing into the typed shape, so a future/unknown major yields
/// a clean [`PlanValidationError::UnsupportedSchemaVersion`] instead of a
/// confusing shape mismatch. On success the returned [`Plan`] has passed every
/// structural rule in [`validate_plan`].
///
/// # Errors
///
/// Returns the first [`PlanValidationError`] found.
pub fn parse_and_validate_plan(raw: &Value) -> Result<Plan, PlanValidationError> {
    let obj = raw.as_object().ok_or(PlanValidationError::NotObject)?;

    // Gate the version first, from the raw value, so an unsupported major is a
    // version error rather than a shape error.
    let version = obj
        .get("schema_version")
        .ok_or(PlanValidationError::SchemaVersionMissing)?;
    let version = version
        .as_u64()
        .ok_or(PlanValidationError::SchemaVersionNotInt)?;
    check_supported_version(version)?;

    // Deserialize into the typed shape. Missing required fields, wrong types,
    // an unknown acceptance `kind`, and an unknown `tier` all fail here.
    let plan: Plan =
        serde_json::from_value(raw.clone()).map_err(|e| PlanValidationError::Malformed {
            message: e.to_string(),
        })?;

    validate_plan(&plan)?;
    Ok(plan)
}

/// Reject a `schema_version` value whose major is not in
/// [`SUPPORTED_PLAN_SCHEMAS`]. Shared by the raw-`Value` gate in
/// [`parse_and_validate_plan`] and the typed re-check in [`validate_plan`], so
/// neither entry point can admit an unsupported major.
fn check_supported_version(version: u64) -> Result<(), PlanValidationError> {
    if u32::try_from(version).is_ok_and(|v| SUPPORTED_PLAN_SCHEMAS.contains(&v)) {
        Ok(())
    } else {
        Err(PlanValidationError::UnsupportedSchemaVersion {
            found: version,
            supported: SUPPORTED_PLAN_SCHEMAS.to_vec(),
        })
    }
}

/// Structural validation of an already-deserialized [`Plan`].
///
/// Enforces every rule the deserializer cannot (design.md §4, §13): no
/// undeclared fields, non-empty required strings, unique chunk ids, resolvable
/// and acyclic `deps`, at least one executable check per chunk and in
/// `acceptance[]`, and safe repo-relative `files_touched` paths. Split out from
/// [`parse_and_validate_plan`] so a caller holding a typed `Plan` (e.g. one it
/// just built) can re-check it without re-serializing.
///
/// # Errors
///
/// Returns the first [`PlanValidationError`] found.
pub fn validate_plan(plan: &Plan) -> Result<(), PlanValidationError> {
    // Re-gate the version: `validate_plan` is a public entry point, and a `Plan`
    // built directly or deserialized without the raw gate could carry an
    // unsupported major. Without this, a caller re-checking a typed plan (as the
    // doc invites) could admit `schema_version: 3`.
    check_supported_version(u64::from(plan.schema_version))?;

    // --- undeclared-field rejection (compatibility semantics) ---
    reject_unknown_fields(&plan.extra, ObjectShape::Plan)?;
    reject_unknown_fields(&plan.feature.extra, ObjectShape::Feature)?;
    reject_unknown_fields(&plan.baseline.extra, ObjectShape::Baseline)?;

    // --- required non-empty strings ---
    non_empty(&plan.feature.slug, "feature.slug")?;
    non_empty(&plan.feature.source_branch, "feature.source_branch")?;
    non_empty(
        &plan.feature.integration_branch,
        "feature.integration_branch",
    )?;
    non_empty(&plan.baseline.r#ref, "baseline.ref")?;
    non_empty(
        &plan.baseline.test_passlist_hash,
        "baseline.test_passlist_hash",
    )?;
    non_empty(
        &plan.baseline.clippy_warnings_hash,
        "baseline.clippy_warnings_hash",
    )?;

    // --- acceptance: non-empty, ≥1 executable check ---
    if plan.acceptance.is_empty() {
        return Err(PlanValidationError::AcceptanceEmpty);
    }
    for (i, item) in plan.acceptance.iter().enumerate() {
        match item {
            Acceptance::Check {
                desc,
                run,
                cwd,
                expect_exit,
            } => {
                non_empty(desc, &format!("acceptance[{i}].desc"))?;
                non_empty(run, &format!("acceptance[{i}].run"))?;
                validate_check_precision(
                    cwd.as_deref(),
                    *expect_exit,
                    &format!("acceptance[{i}]"),
                )?;
            }
            Acceptance::Assertion { desc } => {
                non_empty(desc, &format!("acceptance[{i}].desc"))?;
            }
        }
    }
    if !plan.acceptance.iter().any(Acceptance::is_check) {
        return Err(PlanValidationError::AcceptanceNoCheck);
    }

    // --- chunks: non-empty, unique ids, per-chunk rules ---
    if plan.chunks.is_empty() {
        return Err(PlanValidationError::ChunksEmpty);
    }
    let mut ids: HashSet<&str> = HashSet::with_capacity(plan.chunks.len());
    for chunk in &plan.chunks {
        validate_chunk_id(&chunk.id)?;
        if !ids.insert(chunk.id.as_str()) {
            return Err(PlanValidationError::DuplicateChunkId {
                id: chunk.id.clone(),
            });
        }
    }
    for chunk in &plan.chunks {
        validate_chunk(chunk, &ids)?;
    }

    // --- deps resolvable + DAG acyclic (over the whole graph) ---
    detect_cycle(&plan.chunks)?;

    Ok(())
}

/// Reject any key in `extra` not on `shape`'s [`tolerated_fields`] allowlist —
/// the per-object-shape compatibility check. `path` overrides `shape.label()`
/// when the caller can name the concrete location (e.g. `chunks[c1]`).
fn reject_unknown_fields_at(
    extra: &Map<String, Value>,
    shape: ObjectShape,
    path: &str,
) -> Result<(), PlanValidationError> {
    let allow = tolerated_fields(shape);
    if let Some((field, _)) = extra.iter().find(|(k, _)| !allow.contains(&k.as_str())) {
        return Err(PlanValidationError::UnknownField {
            path: path.to_string(),
            field: field.clone(),
        });
    }
    Ok(())
}

/// [`reject_unknown_fields_at`] using the shape's own label as the error path —
/// for the fixed-location shapes (`Plan`, `feature`, `baseline`).
fn reject_unknown_fields(
    extra: &Map<String, Value>,
    shape: ObjectShape,
) -> Result<(), PlanValidationError> {
    reject_unknown_fields_at(extra, shape, shape.label())
}

/// Reject an empty / whitespace-only required string.
fn non_empty(s: &str, path: &str) -> Result<(), PlanValidationError> {
    if s.trim().is_empty() {
        return Err(PlanValidationError::EmptyString {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// The highest exit status a `sh -c` process can report; a shell truncates the
/// wait status to `0..=255` (a signalled child surfaces as `128 + signal`), so
/// an `expect_exit` outside this range can never match and is rejected.
const MAX_SHELL_EXIT: i32 = 255;

/// Validate a check's optional precision fields (`cwd`, `expect_exit`) — shared
/// by the per-chunk `checks[]` and `acceptance[]` check paths so the two never
/// diverge. `location` is the dotted path to the check (e.g.
/// `chunks[c1].checks[0]` or `acceptance[0]`); the field name is appended here.
///
/// - `cwd`, when present, must be a non-empty *safe repo-relative* directory —
///   the same lexical guard `files_touched` gets ([`is_safe_repo_relative`]),
///   because `cwd` chooses where a shell command runs and an unchecked `/etc` or
///   `../..` would escape the worktree the floor gates. Absence already means the
///   worktree root, so a bare `.` is rejected (one spelling for root).
/// - `expect_exit`, when present, must be a real shell exit status (`0..=255`).
fn validate_check_precision(
    cwd: Option<&str>,
    expect_exit: Option<i32>,
    location: &str,
) -> Result<(), PlanValidationError> {
    if let Some(cwd) = cwd {
        non_empty(cwd, &format!("{location}.cwd"))?;
        if !is_safe_repo_relative(cwd) {
            return Err(PlanValidationError::UnsafeCwd {
                location: format!("{location}.cwd"),
                path: cwd.to_string(),
            });
        }
    }
    if let Some(code) = expect_exit {
        if !(0..=MAX_SHELL_EXIT).contains(&code) {
            return Err(PlanValidationError::ExpectExitOutOfRange {
                location: format!("{location}.expect_exit"),
                value: i64::from(code),
            });
        }
    }
    Ok(())
}

/// Chunk-id shape hint shared by every rejection.
const CHUNK_ID_EXPECTED: &str = "a non-empty id of `[A-Za-z0-9_.-]` starting with an alphanumeric";

/// Validate a chunk id: non-empty, leading alphanumeric, body limited to
/// `[A-Za-z0-9_.-]`. Keeps ids safe to reference in errors, logs, and any
/// future path derived from them (no `/`, `..`, or leading dot).
fn validate_chunk_id(id: &str) -> Result<(), PlanValidationError> {
    let ok = {
        let mut chars = id.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    if ok {
        Ok(())
    } else {
        Err(PlanValidationError::InvalidChunkId {
            id: id.to_string(),
            expected: CHUNK_ID_EXPECTED,
        })
    }
}

/// Per-chunk structural rules (unknown fields, non-empty strings, ≥1 check,
/// declared + safe `files_touched`, resolvable + unique `deps`, non-empty
/// assertions).
fn validate_chunk(chunk: &Chunk, ids: &HashSet<&str>) -> Result<(), PlanValidationError> {
    reject_unknown_fields_at(
        &chunk.extra,
        ObjectShape::Chunk,
        &format!("chunks[{}]", chunk.id),
    )?;
    non_empty(&chunk.title, &format!("chunks[{}].title", chunk.id))?;
    non_empty(&chunk.brief, &format!("chunks[{}].brief", chunk.id))?;

    // deps must resolve to a real chunk (cycles are caught separately) and must
    // not repeat — a duplicate edge is malformed and skews any indegree-based
    // scheduler (a dependent counted twice can never unblock).
    let mut seen_deps: HashSet<&str> = HashSet::with_capacity(chunk.deps.len());
    for dep in &chunk.deps {
        if !ids.contains(dep.as_str()) {
            return Err(PlanValidationError::UnknownDep {
                chunk: chunk.id.clone(),
                dep: dep.clone(),
            });
        }
        if !seen_deps.insert(dep.as_str()) {
            return Err(PlanValidationError::DuplicateDep {
                chunk: chunk.id.clone(),
                dep: dep.clone(),
            });
        }
    }

    // ≥1 executable check.
    if chunk.checks.is_empty() {
        return Err(PlanValidationError::ChunkNoCheck {
            chunk: chunk.id.clone(),
        });
    }
    for (i, check) in chunk.checks.iter().enumerate() {
        reject_unknown_fields_at(
            &check.extra,
            ObjectShape::Check,
            &format!("chunks[{}].checks[{i}]", chunk.id),
        )?;
        non_empty(
            &check.desc,
            &format!("chunks[{}].checks[{i}].desc", chunk.id),
        )?;
        non_empty(&check.run, &format!("chunks[{}].checks[{i}].run", chunk.id))?;
        validate_check_precision(
            check.cwd.as_deref(),
            check.expect_exit,
            &format!("chunks[{}].checks[{i}]", chunk.id),
        )?;
    }

    // assertions are LLM-judged criteria — an empty one is nonsensical (mirrors
    // the non-empty check applied to `acceptance[]` items).
    for (i, assertion) in chunk.assertions.iter().enumerate() {
        non_empty(assertion, &format!("chunks[{}].assertions[{i}]", chunk.id))?;
    }

    // files_touched: declared + safe repo-relative.
    if chunk.files_touched.is_empty() {
        return Err(PlanValidationError::ChunkNoFiles {
            chunk: chunk.id.clone(),
        });
    }
    for path in &chunk.files_touched {
        if !is_safe_repo_relative(path) {
            return Err(PlanValidationError::UnsafePath {
                chunk: chunk.id.clone(),
                path: path.clone(),
            });
        }
    }

    Ok(())
}

/// True iff `p` is a safe repo-relative path. This is a **lexical** guard
/// (mirroring the crate's id-level path-traversal stance in `schema.rs`) applied
/// to multi-component paths — it is deliberately platform-independent, because a
/// plan may be written on one OS and consumed on another. It is NOT a
/// filesystem-resolution guarantee: a lexically-safe path can still resolve
/// outside the repo through a symlinked directory, so the supervisor's actual
/// merge-scope enforcement must not rely on this alone.
///
/// Rejects: empty; absolute (`/…`); `~` home-expansion; backslash (`\`, a
/// Windows separator — kills `\\server\share` too); a `:` anywhere (kills
/// Windows drive/`C:foo` and drive-absolute `C:/…`); any control character
/// (NUL, `\n`, `\r`, `\t` — legal in some filenames but log-poisoning and
/// adversarial); and any component that is empty (`a//b`), whitespace-only,
/// `.` (`a/./b` — a non-canonical form that would defeat file-scope matching),
/// or `..` (traversal).
fn is_safe_repo_relative(p: &str) -> bool {
    if p.is_empty()
        || p.starts_with('/')
        || p.starts_with('~')
        || p.contains('\\')
        || p.contains(':')
        || p.chars().any(char::is_control)
    {
        return false;
    }
    p.split('/')
        .all(|comp| !comp.trim().is_empty() && comp != "." && comp != "..")
}

/// Detect a cycle (or a self-loop) in the chunk dependency graph via a
/// three-colour DFS. On a back-edge to a node still on the DFS stack, returns
/// [`PlanValidationError::DependencyCycle`] with the cycle path (entry id
/// repeated at the end). Assumes every `deps` entry already resolves to a real
/// chunk (checked by [`validate_chunk`]).
fn detect_cycle(chunks: &[Chunk]) -> Result<(), PlanValidationError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Colour {
        White,
        Grey,
        Black,
    }

    let adj: HashMap<&str, &[String]> = chunks
        .iter()
        .map(|c| (c.id.as_str(), c.deps.as_slice()))
        .collect();
    let mut colour: HashMap<&str, Colour> = chunks
        .iter()
        .map(|c| (c.id.as_str(), Colour::White))
        .collect();

    // Iterative DFS with an explicit stack of (node, next-dep-index) so a deep
    // or wide graph cannot blow the call stack. `path` mirrors the grey stack
    // for cycle reconstruction.
    for start in chunks.iter().map(|c| c.id.as_str()) {
        if colour[start] != Colour::White {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        let mut path: Vec<&str> = vec![start];
        colour.insert(start, Colour::Grey);

        while let Some(&mut (node, ref mut idx)) = stack.last_mut() {
            let deps = adj[node];
            if *idx < deps.len() {
                let dep = deps[*idx].as_str();
                *idx += 1;
                match colour[dep] {
                    Colour::White => {
                        colour.insert(dep, Colour::Grey);
                        stack.push((dep, 0));
                        path.push(dep);
                    }
                    Colour::Grey => {
                        // Back-edge: `dep` is an ancestor on the current path.
                        let from = path.iter().position(|&n| n == dep).unwrap_or(0);
                        let mut cycle: Vec<String> =
                            path[from..].iter().map(|s| (*s).to_string()).collect();
                        cycle.push(dep.to_string());
                        return Err(PlanValidationError::DependencyCycle { cycle });
                    }
                    Colour::Black => {}
                }
            } else {
                colour.insert(node, Colour::Black);
                stack.pop();
                path.pop();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The canonical valid plan — the `plan-schema.md` example, kept in sync
    /// with the checked-in `schemas/plan.v2.example.json` by
    /// `checked_in_example_is_valid`.
    fn valid_plan() -> Value {
        serde_json::from_str(include_str!("../schemas/plan.v2.example.json")).unwrap()
    }

    // --- valid ---

    #[test]
    fn example_plan_validates() {
        let plan = parse_and_validate_plan(&valid_plan()).expect("example must validate");
        assert_eq!(plan.schema_version, PLAN_SCHEMA_VERSION);
        assert_eq!(plan.chunks.len(), 2);
        assert_eq!(plan.chunks[1].deps, vec!["c1".to_string()]);
        assert!(plan.chunks[0].requires_tests);
    }

    #[test]
    fn checked_in_example_is_valid() {
        // The example artifact and the doc example are one and the same; if the
        // artifact drifts out of the v2 shape this fails.
        let raw: Value = serde_json::from_str(PLAN_V2_EXAMPLE).unwrap();
        assert!(parse_and_validate_plan(&raw).is_ok());
    }
    const PLAN_V2_EXAMPLE: &str = include_str!("../schemas/plan.v2.example.json");

    #[test]
    fn minimal_valid_plan() {
        let v = json!({
            "schema_version": 2,
            "plan_rev": 1,
            "intent_rev": 1,
            "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
            "baseline": {"ref": "feat/f@fork", "test_passlist_hash": "sha256:a", "clippy_warnings_hash": "sha256:b"},
            "acceptance": [{"kind": "check", "desc": "e2e", "run": "cargo test"}],
            "chunks": [{
                "id": "c1", "title": "t", "tier": "code", "brief": "b",
                "files_touched": ["src/a.rs"],
                "checks": [{"desc": "d", "run": "cargo test a"}]
            }],
        });
        assert!(parse_and_validate_plan(&v).is_ok());
    }

    #[test]
    fn round_trips_through_serde() {
        let plan = parse_and_validate_plan(&valid_plan()).unwrap();
        let reser = serde_json::to_value(&plan).unwrap();
        let again = parse_and_validate_plan(&reser).unwrap();
        assert_eq!(plan, again);
    }

    // --- version gating ---

    #[test]
    fn unsupported_major_rejected() {
        let mut v = valid_plan();
        v["schema_version"] = json!(3);
        let err = parse_and_validate_plan(&v).unwrap_err();
        assert!(matches!(
            err,
            PlanValidationError::UnsupportedSchemaVersion { found: 3, .. }
        ));
        assert_eq!(
            err.expected(),
            Some(json!({"field": "schema_version", "supported": [2]}))
        );
    }

    #[test]
    fn missing_version_rejected() {
        let mut v = valid_plan();
        v.as_object_mut().unwrap().remove("schema_version");
        assert_eq!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::SchemaVersionMissing
        );
    }

    #[test]
    fn non_integer_version_rejected() {
        let mut v = valid_plan();
        v["schema_version"] = json!("2");
        assert_eq!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::SchemaVersionNotInt
        );
    }

    #[test]
    fn validate_plan_regates_version_on_typed_plan() {
        // A typed `Plan` that bypassed the raw gate (built directly, or mutated
        // after deserialization) must still be rejected by `validate_plan` —
        // otherwise the "re-check a typed plan" path admits an unsupported major.
        let mut plan = parse_and_validate_plan(&valid_plan()).unwrap();
        plan.schema_version = 3;
        assert!(matches!(
            validate_plan(&plan).unwrap_err(),
            PlanValidationError::UnsupportedSchemaVersion { found: 3, .. }
        ));
    }

    // --- unknown fields ---

    #[test]
    fn unknown_top_level_field_rejected() {
        let mut v = valid_plan();
        v["budget"] = json!(1000);
        let err = parse_and_validate_plan(&v).unwrap_err();
        assert!(matches!(
            err,
            PlanValidationError::UnknownField { ref field, .. } if field == "budget"
        ));
    }

    #[test]
    fn unknown_chunk_field_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["retries"] = json!(3);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::UnknownField { field, .. } if field == "retries"
        ));
    }

    // --- malformed (deserialize-time) ---

    #[test]
    fn unknown_acceptance_kind_rejected() {
        let mut v = valid_plan();
        v["acceptance"][0]["kind"] = json!("gut-feeling");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    #[test]
    fn unknown_tier_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["tier"] = json!("ultra");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    #[test]
    fn missing_required_chunk_field_rejected() {
        let mut v = valid_plan();
        v["chunks"][0].as_object_mut().unwrap().remove("brief");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    #[test]
    fn non_object_root_rejected() {
        assert_eq!(
            parse_and_validate_plan(&json!([1, 2, 3])).unwrap_err(),
            PlanValidationError::NotObject
        );
    }

    // --- acceptance rules ---

    #[test]
    fn acceptance_all_assertions_rejected() {
        let mut v = valid_plan();
        v["acceptance"] = json!([{"kind": "assertion", "desc": "vibes"}]);
        assert_eq!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::AcceptanceNoCheck
        );
    }

    #[test]
    fn acceptance_empty_rejected() {
        let mut v = valid_plan();
        v["acceptance"] = json!([]);
        assert_eq!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::AcceptanceEmpty
        );
    }

    #[test]
    fn acceptance_check_unknown_field_rejected() {
        // The tagged `Acceptance` enum uses `deny_unknown_fields`, so a stray
        // key inside a variant fails at deserialize time (Malformed), matching
        // the JSON Schema's `additionalProperties: false`. This is the fix for
        // the silent-drop divergence all reviewers flagged.
        let mut v = valid_plan();
        v["acceptance"][0]["budget"] = json!(100);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    #[test]
    fn acceptance_assertion_with_run_rejected() {
        // `run` is not a field of the `assertion` variant — reject it rather
        // than silently drop an executable command onto a non-executable item.
        let mut v = valid_plan();
        v["acceptance"] = json!([
            {"kind": "check", "desc": "e2e", "run": "cargo test"},
            {"kind": "assertion", "desc": "x", "run": "rm -rf /"},
        ]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    // --- chunk rules ---

    #[test]
    fn chunk_missing_check_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["checks"] = json!([]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::ChunkNoCheck { chunk } if chunk == "c1"
        ));
    }

    #[test]
    fn chunk_empty_files_touched_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["files_touched"] = json!([]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::ChunkNoFiles { chunk } if chunk == "c1"
        ));
    }

    #[test]
    fn duplicate_chunk_id_rejected() {
        let mut v = valid_plan();
        v["chunks"][1]["id"] = json!("c1");
        // dep "c1" still resolves; the duplicate id is what fails.
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::DuplicateChunkId { id } if id == "c1"
        ));
    }

    #[test]
    fn dangling_dep_rejected() {
        let mut v = valid_plan();
        v["chunks"][1]["deps"] = json!(["nope"]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::UnknownDep { dep, .. } if dep == "nope"
        ));
    }

    #[test]
    fn invalid_chunk_id_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["id"] = json!("../evil");
        // deps still point at "c1"; make c2 independent so the id check fires.
        v["chunks"][1]["deps"] = json!([]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::InvalidChunkId { .. }
        ));
    }

    #[test]
    fn duplicate_dep_rejected() {
        let mut v = valid_plan();
        v["chunks"][1]["deps"] = json!(["c1", "c1"]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::DuplicateDep { dep, .. } if dep == "c1"
        ));
    }

    #[test]
    fn empty_chunk_assertion_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["assertions"] = json!(["ok", "   "]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::EmptyString { path } if path == "chunks[c1].assertions[1]"
        ));
    }

    // --- flexible check shape (desc + run + optional cwd/expect_exit) ---

    #[test]
    fn check_with_cwd_and_expect_exit_validates_and_round_trips() {
        let mut v = valid_plan();
        v["chunks"][0]["checks"] = json!([
            {"desc": "runs in a subdir with a non-zero expected code",
             "run": "make check", "cwd": "crates/x", "expect_exit": 2},
        ]);
        v["acceptance"] = json!([
            {"kind": "check", "desc": "e2e", "run": "cargo test", "cwd": "tests", "expect_exit": 0},
        ]);
        let plan = parse_and_validate_plan(&v).expect("optional check fields must validate");

        // The optional fields land on the typed shape, not in `extra`.
        let check = &plan.chunks[0].checks[0];
        assert_eq!(check.cwd.as_deref(), Some("crates/x"));
        assert_eq!(check.expect_exit, Some(2));
        assert!(check.extra.is_empty());
        assert!(matches!(
            &plan.acceptance[0],
            Acceptance::Check { cwd, expect_exit, .. }
                if cwd.as_deref() == Some("tests") && *expect_exit == Some(0)
        ));

        // Round-trips through serde back to an equal, still-valid plan.
        let reser = serde_json::to_value(&plan).unwrap();
        assert_eq!(parse_and_validate_plan(&reser).unwrap(), plan);
    }

    #[test]
    fn check_without_optional_fields_defaults() {
        // Back-compat: a check with only desc+run parses, leaving the optional
        // precision absent (expect_exit defaults to 0 at execution time). The
        // absent fields skip serialization entirely.
        let plan = parse_and_validate_plan(&valid_plan()).unwrap();
        let check = &plan.chunks[0].checks[0];
        assert_eq!(check.cwd, None);
        assert_eq!(check.expect_exit, None);

        let reser = serde_json::to_value(&plan.chunks[0].checks[0]).unwrap();
        let obj = reser.as_object().unwrap();
        assert!(!obj.contains_key("cwd"));
        assert!(!obj.contains_key("expect_exit"));
    }

    #[test]
    fn empty_check_cwd_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["checks"][0]["cwd"] = json!("  ");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::EmptyString { path } if path == "chunks[c1].checks[0].cwd"
        ));
    }

    #[test]
    fn empty_acceptance_check_cwd_rejected() {
        let mut v = valid_plan();
        v["acceptance"][0]["cwd"] = json!("");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::EmptyString { path } if path == "acceptance[0].cwd"
        ));
    }

    #[test]
    fn non_integer_expect_exit_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["checks"][0]["expect_exit"] = json!("nope");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::Malformed { .. }
        ));
    }

    #[test]
    fn unsafe_check_cwd_rejected() {
        // `cwd` controls where a shell command runs, so it gets the same
        // repo-relative safety guard as `files_touched` — an absolute path or a
        // `..`/`~` traversal would let a check escape the worktree the floor
        // gates. A bare `.` is rejected too: absence already means the root.
        for bad in [
            "/etc",
            "../../outside",
            "a/../../etc",
            "~/secret",
            ".",
            "a\\b",
        ] {
            let mut v = valid_plan();
            v["chunks"][0]["checks"][0]["cwd"] = json!(bad);
            assert!(
                matches!(
                    parse_and_validate_plan(&v).unwrap_err(),
                    PlanValidationError::UnsafeCwd { location, .. }
                        if location == "chunks[c1].checks[0].cwd"
                ),
                "expected UnsafeCwd for chunk cwd {bad:?}"
            );
        }
    }

    #[test]
    fn unsafe_acceptance_check_cwd_rejected() {
        for bad in ["/etc", "../escape", "~/x", "."] {
            let mut v = valid_plan();
            v["acceptance"][0]["cwd"] = json!(bad);
            assert!(
                matches!(
                    parse_and_validate_plan(&v).unwrap_err(),
                    PlanValidationError::UnsafeCwd { location, .. }
                        if location == "acceptance[0].cwd"
                ),
                "expected UnsafeCwd for acceptance cwd {bad:?}"
            );
        }
    }

    #[test]
    fn out_of_range_expect_exit_rejected() {
        // A shell exit status is 0..=255; anything outside can never match
        // `code()` and would make the check permanently un-passable.
        for (loc, patch) in [
            (
                "chunks[c1].checks[0].expect_exit",
                (&["chunks", "0", "checks", "0"][..], -1),
            ),
            (
                "chunks[c1].checks[0].expect_exit",
                (&["chunks", "0", "checks", "0"][..], 256),
            ),
            ("acceptance[0].expect_exit", (&["acceptance", "0"][..], 300)),
        ] {
            let mut v = valid_plan();
            let (path, code) = patch;
            let mut node = &mut v;
            for key in path {
                node = match key.parse::<usize>() {
                    Ok(idx) => &mut node[idx],
                    Err(_) => &mut node[key],
                };
            }
            node["expect_exit"] = json!(code);
            assert!(
                matches!(
                    parse_and_validate_plan(&v).unwrap_err(),
                    PlanValidationError::ExpectExitOutOfRange { location, value }
                        if location == loc && value == i64::from(code)
                ),
                "expected ExpectExitOutOfRange for {loc} = {code}"
            );
        }
    }

    #[test]
    fn boundary_expect_exit_accepted() {
        // 0 and 255 are the inclusive bounds — both valid.
        for code in [0, 255] {
            let mut v = valid_plan();
            v["chunks"][0]["checks"][0]["expect_exit"] = json!(code);
            assert!(
                parse_and_validate_plan(&v).is_ok(),
                "expect_exit {code} should be accepted"
            );
        }
    }

    // --- DAG acyclicity ---

    #[test]
    fn self_loop_rejected() {
        let mut v = valid_plan();
        v["chunks"][0]["deps"] = json!(["c1"]);
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::DependencyCycle { .. }
        ));
    }

    #[test]
    fn two_cycle_rejected() {
        let mut v = valid_plan();
        // c1 -> c2 and c2 -> c1.
        v["chunks"][0]["deps"] = json!(["c2"]);
        v["chunks"][1]["deps"] = json!(["c1"]);
        let err = parse_and_validate_plan(&v).unwrap_err();
        match err {
            PlanValidationError::DependencyCycle { cycle } => {
                assert_eq!(cycle.first(), cycle.last());
                assert!(cycle.contains(&"c1".to_string()));
                assert!(cycle.contains(&"c2".to_string()));
            }
            other => panic!("expected cycle, got {other:?}"),
        }
    }

    #[test]
    fn longer_cycle_rejected() {
        // Three chunks a -> b -> c -> a.
        let v = json!({
            "schema_version": 2, "plan_rev": 1, "intent_rev": 1,
            "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
            "baseline": {"ref": "r", "test_passlist_hash": "h", "clippy_warnings_hash": "h"},
            "acceptance": [{"kind": "check", "desc": "e2e", "run": "t"}],
            "chunks": [
                {"id": "a", "title": "t", "tier": "code", "brief": "b", "deps": ["c"], "files_touched": ["x"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "b", "title": "t", "tier": "code", "brief": "b", "deps": ["a"], "files_touched": ["y"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "c", "title": "t", "tier": "code", "brief": "b", "deps": ["b"], "files_touched": ["z"], "checks": [{"desc": "d", "run": "r"}]},
            ],
        });
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::DependencyCycle { .. }
        ));
    }

    #[test]
    fn diamond_dag_is_acyclic() {
        // a -> {b, c} -> d is a valid DAG (a shared dep + a join).
        let v = json!({
            "schema_version": 2, "plan_rev": 1, "intent_rev": 1,
            "feature": {"slug": "f", "source_branch": "main", "integration_branch": "feat/f"},
            "baseline": {"ref": "r", "test_passlist_hash": "h", "clippy_warnings_hash": "h"},
            "acceptance": [{"kind": "check", "desc": "e2e", "run": "t"}],
            "chunks": [
                {"id": "a", "title": "t", "tier": "code", "brief": "b", "files_touched": ["w"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "b", "title": "t", "tier": "code", "brief": "b", "deps": ["a"], "files_touched": ["x"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "c", "title": "t", "tier": "code", "brief": "b", "deps": ["a"], "files_touched": ["y"], "checks": [{"desc": "d", "run": "r"}]},
                {"id": "d", "title": "t", "tier": "code", "brief": "b", "deps": ["b", "c"], "files_touched": ["z"], "checks": [{"desc": "d", "run": "r"}]},
            ],
        });
        assert!(parse_and_validate_plan(&v).is_ok());
    }

    // --- path traversal ---

    #[test]
    fn path_traversal_in_files_touched_rejected() {
        for bad in [
            "../etc/passwd",
            "/abs/path",
            "~/secret",
            "a/../b",
            "a//b",
            "a\\b",
        ] {
            let mut v = valid_plan();
            v["chunks"][0]["files_touched"] = json!([bad]);
            assert!(
                matches!(
                    parse_and_validate_plan(&v).unwrap_err(),
                    PlanValidationError::UnsafePath { .. }
                ),
                "expected UnsafePath for {bad:?}"
            );
        }
    }

    #[test]
    fn safe_paths_accepted() {
        for ok in [
            "src/a.rs",
            "crates/x/src/mod.rs",
            "a.rs",
            "deep/nested/dir/file.txt",
            ".github/workflows/ci.yml", // leading-dot dir is fine; only `.`/`..` components are rejected
        ] {
            assert!(is_safe_repo_relative(ok), "should accept {ok:?}");
        }
        for bad in [
            "",             // empty
            "/x",           // absolute
            "~/x",          // home expansion
            "..",           // traversal
            "a/../b",       // traversal component
            "a//b",         // empty component
            "a\\b",         // backslash separator
            "a/./b",        // non-canonical `.` component
            ".",            // bare `.`
            "C:/Windows",   // windows drive-absolute (colon)
            "C:foo",        // windows drive-relative (colon)
            "src/foo\nbar", // control char (log poisoning)
            "src/foo\tbar", // control char
            "   ",          // whitespace-only
            "a/   /b",      // whitespace-only component
        ] {
            assert!(!is_safe_repo_relative(bad), "should reject {bad:?}");
        }
    }

    // --- empty required strings ---

    #[test]
    fn empty_feature_slug_rejected() {
        let mut v = valid_plan();
        v["feature"]["slug"] = json!("   ");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::EmptyString { path } if path == "feature.slug"
        ));
    }

    #[test]
    fn empty_baseline_hash_rejected() {
        let mut v = valid_plan();
        v["baseline"]["test_passlist_hash"] = json!("");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::EmptyString { path } if path == "baseline.test_passlist_hash"
        ));
    }

    // --- tier wire names ---

    #[test]
    fn tier_wire_names_round_trip() {
        for &name in Tier::WIRE_NAMES {
            let tier: Tier = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(serde_json::to_value(tier).unwrap(), json!(name));
        }
    }

    // --- JSON Schema drift guard ---

    #[test]
    fn json_schema_matches_rust_types() {
        let schema: Value = serde_json::from_str(PLAN_V2_JSON_SCHEMA)
            .expect("checked-in JSON Schema must be valid JSON");

        // Version constant agrees.
        assert_eq!(
            schema["properties"]["schema_version"]["const"],
            json!(PLAN_SCHEMA_VERSION)
        );

        // Required top-level fields agree with the Rust struct's fields.
        let required: HashSet<String> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let expected: HashSet<String> = [
            "schema_version",
            "plan_rev",
            "intent_rev",
            "feature",
            "baseline",
            "acceptance",
            "chunks",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(required, expected);

        // Tier enum agrees.
        let tiers: Vec<String> = schema["$defs"]["chunk"]["properties"]["tier"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(tiers, Tier::WIRE_NAMES);

        // Nested required-field sets agree with the Rust structs.
        let required_at = |ptr: &str| -> HashSet<String> {
            schema
                .pointer(ptr)
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("missing required[] at {ptr}"))
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        };
        let set = |fields: &[&str]| -> HashSet<String> {
            fields.iter().map(ToString::to_string).collect()
        };
        assert_eq!(
            required_at("/properties/feature/required"),
            set(&["slug", "source_branch", "integration_branch"])
        );
        assert_eq!(
            required_at("/properties/baseline/required"),
            set(&["ref", "test_passlist_hash", "clippy_warnings_hash"])
        );
        assert_eq!(
            required_at("/$defs/chunk/required"),
            set(&["id", "title", "tier", "brief", "files_touched", "checks"])
        );
        assert_eq!(required_at("/$defs/check/required"), set(&["desc", "run"]));

        // Acceptance variants keep their exact required-sets — the `check` arm
        // requires `kind`+`desc`+`run` (never the optional precision), the
        // `assertion` arm `kind`+`desc`. A future edit that promoted `cwd`/
        // `expect_exit` to required would diverge from the Rust `Option<_>`.
        assert_eq!(
            required_at("/$defs/acceptance_item/oneOf/0/required"),
            set(&["kind", "desc", "run"])
        );
        assert_eq!(
            required_at("/$defs/acceptance_item/oneOf/1/required"),
            set(&["kind", "desc"])
        );

        // The flexible-check optional fields (`plan-check-run-contract`) are
        // present as optional (not required) properties on both the per-chunk
        // check def and the acceptance `check` variant — mirroring the Rust
        // `Option<_>` fields on `Check` / `Acceptance::Check`. The schema-side
        // constraints must also match the Rust validator: `cwd` non-empty
        // (`minLength: 1`) and `expect_exit` bounded to the shell range
        // `0..=255`. If a future edit drops or loosens either, schema and types
        // stop agreeing and this fails.
        for ptr in [
            "/$defs/check/properties",
            "/$defs/acceptance_item/oneOf/0/properties",
        ] {
            let props = schema
                .pointer(ptr)
                .unwrap_or_else(|| panic!("missing {ptr}"));
            assert_eq!(
                props["cwd"]["type"],
                json!("string"),
                "expected optional string `cwd` at {ptr}"
            );
            assert_eq!(
                props["cwd"]["minLength"],
                json!(1),
                "expected `cwd` minLength:1 at {ptr}"
            );
            assert_eq!(
                props["expect_exit"]["type"],
                json!("integer"),
                "expected optional integer `expect_exit` at {ptr}"
            );
            assert_eq!(
                props["expect_exit"]["minimum"],
                json!(0),
                "expected `expect_exit` minimum:0 at {ptr}"
            );
            assert_eq!(
                props["expect_exit"]["maximum"],
                json!(i64::from(MAX_SHELL_EXIT)),
                "expected `expect_exit` maximum:255 at {ptr}"
            );
        }

        // Every object shape closes itself with `additionalProperties: false` —
        // the schema-side mirror of the Rust reject-unknown-fields policy. If a
        // future edit drops one, the two stop agreeing and this fails.
        for ptr in [
            "",
            "/properties/feature",
            "/properties/baseline",
            "/$defs/chunk",
            "/$defs/check",
            "/$defs/acceptance_item/oneOf/0",
            "/$defs/acceptance_item/oneOf/1",
        ] {
            let node = if ptr.is_empty() {
                &schema
            } else {
                schema
                    .pointer(ptr)
                    .unwrap_or_else(|| panic!("missing {ptr}"))
            };
            assert_eq!(
                node["additionalProperties"],
                json!(false),
                "expected additionalProperties:false at {ptr:?}"
            );
        }

        // Acceptance `kind` discriminants agree with the Rust enum wire names.
        let kinds: HashSet<String> = schema["$defs"]["acceptance_item"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|variant| {
                variant["properties"]["kind"]["const"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(kinds, set(&["check", "assertion"]));

        // The example the doc/tests use validates against the Rust validator,
        // tying schema + types + example together.
        let example: Value = serde_json::from_str(PLAN_V2_EXAMPLE).unwrap();
        assert!(parse_and_validate_plan(&example).is_ok());
    }

    #[test]
    fn tolerated_optional_seam_is_empty_in_v2() {
        // The governed-evolution seam exists but admits nothing in v2: every
        // object shape's allowlist is empty, so any unknown key is rejected.
        for shape in [
            ObjectShape::Plan,
            ObjectShape::Feature,
            ObjectShape::Baseline,
            ObjectShape::Chunk,
            ObjectShape::Check,
        ] {
            assert!(tolerated_fields(shape).is_empty());
        }
        assert!(TOLERATED_OPTIONAL_FIELDS.is_empty());
    }

    #[test]
    fn unknown_field_scoped_to_its_object() {
        // A per-shape allowlist means an unknown key is reported against the
        // object that carries it, not conflated across shapes.
        let mut v = valid_plan();
        v["feature"]["team"] = json!("payments");
        assert!(matches!(
            parse_and_validate_plan(&v).unwrap_err(),
            PlanValidationError::UnknownField { path, field }
                if path == "feature" && field == "team"
        ));
    }
}
