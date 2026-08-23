---
created: 2026-08-23
updated: 2026-08-23
type: bug
status: in-progress
priority: high
lane: release
lane_seq: 20
collision: [scripts/ossctl-release.sh]
---

# Release wrapper rejects ossctl 0.10.1

## Description

## Description

The owned-CLI release sweep found genuinely releasable orchestratectl patch content for v0.5.1, but `scripts/ossctl-release.sh` fails closed because it admits only ossctl 0.10.0 commit `a35b9917…` while the fleet-managed installed release is ossctl 0.10.1.

The wrapper's version pin is intentionally a protocol safety gate. Revalidate 0.10.1 against every held-tag, journal, exact-SHA CI, resume, and verify assumption before widening it. Repository policy grants fully autonomous release cuts once the wrapper and exact plan are valid; there is no human approval gate.

## Acceptance Criteria

- Revalidate installed ossctl 0.10.1's `release plan/cut/show/list/resume/verify` JSON and held pre-tag semantics against the wrapper.
- Admit exactly the verified 0.10.1 build/protocol while preserving fail-closed rejection of unsupported builds and unsafe near-miss states.
- Add deterministic wrapper tests for the 0.10.1 identity and protocol behavior.
- Run the complete repository green gate and `/llm-review` plus `/assess-findings`.
- Do not cut or publish v0.5.1 from the worker. The integrated-main orchestrator will run the full gate, seal a fresh patch plan, and execute the autonomous cut after this lands.
