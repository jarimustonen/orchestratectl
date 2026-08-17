---
created: 2026-08-17
updated: 2026-08-17
type: chore
reporter: jari
status: in-progress
priority: normal
lane: core
lane_seq: 10
---

# Cut the dead plan module from octl-core's public API

_Source: crates/octl-core/src/plan.rs_

## Description

octl-core/src/plan.rs (2013 lines: Plan v3 schema, parse_and_validate_plan, PLAN_V3_JSON_SCHEMA, Tier/Chunk/Feature/etc.) belonged to the code-pipeline that DECISION-1 cut on 2026-08-14 (cut-pipeline-floor-harness-heavy). It has zero consumers in octl-cli or anywhere else, but is still re-exported from lib.rs, so the published octl-core crate's public API carries a dead feature. Remove plan.rs and its lib.rs re-exports. CAUTION: this is a symbol-removing cut — per AGENTS.md the green gate MUST include RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace (dangling intra-doc links to removed symbols are only caught there), plus the insta snapshot loop if any surface strings change. Also grep docs/ and issues/*/design.md are NOT in scope (historical docs stay).
