---
created: 2026-07-22
updated: 2026-08-17
type: epic
status: obsolete
priority: high
closed: 2026-08-17
---

# Code Pipeline — spec-driven, model-tiered coding

## Description


The spec-driven, model-tiered code pipeline (plan v3 schema, floor/pipeline
modules, harness heavy layer). Implemented as a walking skeleton in July 2026,
then **cut by DECISION-1** (2026-08-14, `cut-pipeline-floor-harness-heavy`):
the 0.2 simplification removed `crates/taskfleet-cli/src/{floor,pipeline}/*` and the
heavy harness layer. The last remnant, the dead `taskfleet-core/src/plan.rs` module,
is tracked as `@cut-plan-module`.

## Issues

- [x] `floor-capture-hardening-round-2` — done
- [x] `floor-capture-hardening-round-3` — done
- [x] `floor-capture-trust-model` — done
- [x] `pipeline-drop-primitive-underspecified` — obsolete
- [x] `plan-check-run-contract` — fixed
- [x] `plan-schema-v3-provenance-required` — done

## Close note (2026-08-17)

Closed **obsolete**: the pipeline was cut by DECISION-1 / the thin-supervisor
0.2 release; design docs under this directory are historical. Do not
resurrect without a new decision (see `docs/decisions/0001`).
