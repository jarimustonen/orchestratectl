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
//!   rejection ([`PlanValidationError::UnknownField`]), *except* names in
//!   [`TOLERATED_OPTIONAL_FIELDS`]. That allowlist is the governed-evolution
//!   seam: when a future minor adds a genuinely additive *optional* field it is
//!   registered there so older readers tolerate it, and only then. Schema
//!   growth otherwise goes gap-event → reviewed proposal → versioned schema.
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
/// rejection. A future minor that adds a genuinely additive optional field
/// registers its name here (and in the JSON Schema) so older readers tolerate
/// it; anything not listed is treated as a possibly-required unknown and
/// rejected.
pub const TOLERATED_OPTIONAL_FIELDS: &[&str] = &[];

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Acceptance {
    /// An executable end-to-end check (`desc` + shell/test `run`).
    Check {
        /// Human-readable description of what the check proves.
        desc: String,
        /// The command/test invocation the supervisor executes.
        run: String,
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

/// An executable check: a description plus the command/test invocation that
/// proves it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    /// Human-readable description of what the check proves.
    pub desc: String,
    /// The command/test invocation the supervisor executes.
    pub run: String,
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
    #[error("path {path:?} in chunk {chunk:?} is not a safe repo-relative path (no absolute paths, `..`, `~`, or empty components)")]
    UnsafePath {
        /// The offending chunk id.
        chunk: String,
        /// The offending path.
        path: String,
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
    if !u32::try_from(version).is_ok_and(|v| SUPPORTED_PLAN_SCHEMAS.contains(&v)) {
        return Err(PlanValidationError::UnsupportedSchemaVersion {
            found: version,
            supported: SUPPORTED_PLAN_SCHEMAS.to_vec(),
        });
    }

    // Deserialize into the typed shape. Missing required fields, wrong types,
    // an unknown acceptance `kind`, and an unknown `tier` all fail here.
    let plan: Plan =
        serde_json::from_value(raw.clone()).map_err(|e| PlanValidationError::Malformed {
            message: e.to_string(),
        })?;

    validate_plan(&plan)?;
    Ok(plan)
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
    // --- undeclared-field rejection (compatibility semantics) ---
    reject_unknown_fields(&plan.extra, "")?;
    reject_unknown_fields(&plan.feature.extra, "feature")?;
    reject_unknown_fields(&plan.baseline.extra, "baseline")?;

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
            Acceptance::Check { desc, run } => {
                non_empty(desc, &format!("acceptance[{i}].desc"))?;
                non_empty(run, &format!("acceptance[{i}].run"))?;
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

/// Reject any key in `extra` not on the [`TOLERATED_OPTIONAL_FIELDS`] allowlist.
/// `path` is the dotted location of the object (empty for the top-level plan).
fn reject_unknown_fields(
    extra: &Map<String, Value>,
    path: &str,
) -> Result<(), PlanValidationError> {
    if let Some((field, _)) = extra
        .iter()
        .find(|(k, _)| !TOLERATED_OPTIONAL_FIELDS.contains(&k.as_str()))
    {
        return Err(PlanValidationError::UnknownField {
            path: if path.is_empty() {
                "<plan>".to_string()
            } else {
                path.to_string()
            },
            field: field.clone(),
        });
    }
    Ok(())
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
/// declared + safe `files_touched`, resolvable `deps`).
fn validate_chunk(chunk: &Chunk, ids: &HashSet<&str>) -> Result<(), PlanValidationError> {
    reject_unknown_fields(&chunk.extra, &format!("chunks[{}]", chunk.id))?;
    non_empty(&chunk.title, &format!("chunks[{}].title", chunk.id))?;
    non_empty(&chunk.brief, &format!("chunks[{}].brief", chunk.id))?;

    // deps must resolve to a real chunk (cycles are caught separately).
    for dep in &chunk.deps {
        if !ids.contains(dep.as_str()) {
            return Err(PlanValidationError::UnknownDep {
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
        reject_unknown_fields(&check.extra, &format!("chunks[{}].checks[{i}]", chunk.id))?;
        non_empty(
            &check.desc,
            &format!("chunks[{}].checks[{i}].desc", chunk.id),
        )?;
        non_empty(&check.run, &format!("chunks[{}].checks[{i}].run", chunk.id))?;
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

/// True iff `p` is a safe repo-relative path: non-empty, not absolute, no `~`
/// home-expansion, no `..` traversal component, no empty components (`a//b`),
/// and no backslash or NUL. Mirrors the crate's id-level path-traversal stance
/// (see `schema.rs` id newtypes) applied to multi-component paths.
fn is_safe_repo_relative(p: &str) -> bool {
    if p.is_empty()
        || p.starts_with('/')
        || p.starts_with('~')
        || p.contains('\\')
        || p.contains('\0')
    {
        return false;
    }
    p.split('/').all(|comp| !comp.is_empty() && comp != "..")
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
        ] {
            assert!(is_safe_repo_relative(ok), "should accept {ok:?}");
        }
        for bad in ["", "/x", "~/x", "..", "a/../b", "a//b", "a\\b"] {
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

        // The example the doc/tests use validates against the Rust validator,
        // tying schema + types + example together.
        let example: Value = serde_json::from_str(PLAN_V2_EXAMPLE).unwrap();
        assert!(parse_and_validate_plan(&example).is_ok());
    }
}
