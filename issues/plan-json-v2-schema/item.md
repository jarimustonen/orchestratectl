---
created: 2026-07-22
updated: 2026-07-22
type: task
status: fixed
priority: high
closed: 2026-07-22
commits:
- hash: bb84fb4
  summary: plan.json v2 types+validator+schema
- hash: 2c99c69
  summary: address llm-review findings
---

# plan.json v2 schema + validator (checks/assertions, immutable plan_rev, intent_rev, DAG validity)

## Description

Turn `plan-schema.md` v2 into a checked-in machine-readable schema and a Rust
type + validator the supervisor/spec-node consume (T2 of the `code-pipeline`
epic). Behavior-preserving: a new `taskfleet_core::plan` module + checked-in schema
artifact, not wired into any live path.

Delivered:

- **Serde types** (`Plan`, `Feature`, `Baseline`, `Acceptance` check/assertion
  tagged enum, `Chunk`, `Check`, `Tier`) mirroring `plan-schema.md` v2 exactly,
  in `crates/taskfleet-core/src/plan.rs`.
- **Validator** (`parse_and_validate_plan` / `validate_plan`) with domain-typed
  `PlanValidationError`s: rejects unsupported `schema_version` majors and
  undeclared fields (allowlist seam for future additive optionals), enforces
  unique chunk ids, resolvable `deps`, DAG acyclicity, ≥1 executable check per
  chunk and in `acceptance[]`, declared + safe repo-relative `files_touched`,
  and known `tier` values.
- **Checked-in JSON Schema** (`crates/taskfleet-core/schemas/plan.v2.schema.json`,
  Draft 2020-12) + example (`plan.v2.example.json`), kept in sync with the Rust
  types by a drift-guard golden test.
- Golden valid + malformed-rejection tests (cycle, missing check, dup id, bad
  version, path traversal, unknown field, …); CHANGELOG entry.

Under-specified in `plan-schema.md` and filed as a follow-up: the exact
`check.run` execution contract (see issue `plan-check-run-contract`).
