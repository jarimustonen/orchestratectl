---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: done
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: 7c5a1a8
  summary: 'feat(report): typed node.report origin instead of string sniffing'
- hash: f695979
  summary: 'fix(outcome): don''t downgrade a malformed origin to the legacy via path'
closed: 2026-08-15
---

# Type node.report provenance instead of reason/via string sniffing

## Description

From /llm-review of A6. supervise::outcome::is_supervisor_failure classifies Blocked vs Failed by reason.starts_with("agent-") + a hard-coded reason list, and TerminalOutcome::Merged trusts via:"explicit-merge" from any report author. Teardown is unaffected (Blocked/Failed both -> PreserveWork; a spoofed merge marker still needs success:true and the reducer rejects contradictions), so this is observability/robustness not invariant-5. Persist a typed report origin (Agent | Supervisor | RunMerge{op_id,worker_oid}) on the node.report event and have classify read the typed field.

## Acceptance Criteria

- [x] A typed `ReportOrigin` (`Agent | Supervisor | RunMerge{op_id, worker_oid}`) is persisted on `node.report` payloads under an `origin` key (`octl_core::report`).
- [x] `run merge` / its crash recovery stamp `RunMerge` (sole merge authority); the supervisor stamps `Supervisor` on every synthesized failure; `node report` normalizes any caller-supplied origin to `Agent`.
- [x] `supervise::outcome::classify` / `is_supervisor_failure` read the typed origin, with the legacy `via`/`reason` sniff gated on the origin field being genuinely ABSENT (a malformed origin never re-unlocks the legacy path).
- [x] Legacy on-disk reports (no origin field) decode read-only and classify exactly as before.
- [x] Merge authorization stays tied to the run-merge path — an `Agent`-origin report cannot classify as Merged even with a forged `via`.
- [x] Regression tests for origin classification, malformed-origin non-downgrade, legacy compat, and agent-origin normalization; full green gate (fmt/clippy/test/doc).
- [x] `/llm-review` + assessment run; confirmed localized fix applied (malformed-origin gating); out-of-scope findings deferred to `retire-via-string`.
