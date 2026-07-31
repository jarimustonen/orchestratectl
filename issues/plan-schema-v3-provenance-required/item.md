---
created: 2026-07-31
updated: 2026-07-31
type: improvement
status: open
priority: normal
epic: code-pipeline
---

## Description

Follow-up to `floor-capture-hardening-round-3` (item 5, deferred sub-part).

Round-3 wired `verify_plan_baseline` into the T5 evaluator (`gate_plan_baseline` in `crates/octl-cli/src/pipeline/live/mod.rs`), which already fails a run closed on empty/missing baseline provenance and validates the OID shape + toolchain (semver-tolerant, rejects `unknown`). The remaining sub-part — bumping the plan schema so `commit_oid` / `toolchain` / `enumerated_targets_hash` are **structurally required** at `validate_plan` rather than silently `#[serde(default)]` — was deferred because it cascades into a public-API artifact migration:

- `PLAN_SCHEMA_VERSION` 2 -> 3, `SUPPORTED_PLAN_SCHEMAS`, and a `PROVENANCE_REQUIRED_SCHEMA` gate in `validate_plan`.
- A new `crates/octl-core/schemas/plan.v3.schema.json` + `plan.v3.example.json` (`json_schema_matches_rust_types` couples the checked-in JSON Schema `const` to `PLAN_SCHEMA_VERSION`).
- The public `plan_v2_*` consts/fns (`PLAN_V2_JSON_SCHEMA`, `PLAN_V2_EXAMPLE`, `plan_v2_json_schema()`) — either rename to v3 (breaks the public API + consumers) or keep v2-named while writing v3 (inconsistent).
- Several `plan.rs` inline fixtures + the unsupported-major / regates tests need updating.

Marginal security value over the now-wired `verify_plan_baseline` gate is low (structural presence vs runtime fail-closed), so it was split out. Do the full v3 migration here.
# Plan schema v3: make baseline provenance structurally required

## Description

