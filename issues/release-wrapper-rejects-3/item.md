---
created: 2026-08-23
updated: 2026-08-23
type: bug
status: fixed
priority: high
lane: release
lane_seq: 20
collision: [scripts/ossctl-release.sh]
commits:
- hash: 1d2e6fb2bb498bdf08ac6b2ced0d2bd68c880aa8
  summary: 'fix: validate ossctl 0.10.1 release protocol'
- hash: b1a45210062b37d95230cde8983d131df4731c44
  summary: 'test: harden ossctl protocol validation'
- hash: b5b89c2136c892ac9fe7a5be009e78ac791f83fd
  summary: 'test: exercise ossctl resume and verify protocol'
- hash: 890314995b8502986aa964279a531aeca28c5b7f
  summary: 'chore: record release wrapper validation commit'
closed: 2026-08-23
---

# Release wrapper rejects ossctl 0.10.1

## Description

The owned-CLI release sweep found genuinely releasable taskfleet patch content for v0.5.1, but `scripts/ossctl-release.sh` fails closed because it admits only ossctl 0.10.0 commit `a35b9917…` while the fleet-managed installed release is ossctl 0.10.1.

The wrapper's version pin is intentionally a protocol safety gate. Revalidate 0.10.1 against every held-tag, journal, exact-SHA CI, resume, and verify assumption before widening it. Repository policy grants fully autonomous release cuts once the wrapper and exact plan are valid; there is no human approval gate.

## Acceptance Criteria

- [x] Revalidate installed ossctl 0.10.1's `release plan/cut/show/list/resume/verify` JSON and held pre-tag semantics against the wrapper.
- [x] Admit exactly the verified 0.10.1 build/protocol while preserving fail-closed rejection of unsupported builds and unsafe near-miss states.
- [x] Add deterministic wrapper tests for the 0.10.1 identity and protocol behavior.
- [x] Run the complete repository green gate and `/llm-review` plus `/assess-findings`.
- [x] Do not cut or publish v0.5.1 from the worker. The integrated-main orchestrator will run the full gate, seal a fresh patch plan, and execute the autonomous cut after this lands.

## Resolution

### 2026-08-23T12:32:45Z · @issuectl

Validated exact fleet ossctl 0.10.1 commit 6879e040a520a7a9c6196ed77791b4f2f10ad6f4 through isolated held-tag, local-only resume, delegated-failure verify, and deterministic near-miss tests. Applied all six confirmed LLM review fixes. The complete required repository gate passed; v0.5.1 was not cut or published by this worker.
