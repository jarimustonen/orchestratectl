---
created: 2026-06-12
updated: 2026-06-27
type: feature
status: done
priority: normal
epic: orchestratectl-mvp
---

# Move §7.3 report validator into octl-core

## Description


`validate_report_payload` lives in `crates/octl-cli/src/node/report.rs` and returns `CliError` directly. When `supervisor-process` lands and needs to validate child reports before consuming them (§7.3 step 3), it cannot call the same function — it would have to copy it or pull the CLI as a dependency.

Move the validator and its sub-validators to `octl_core::report` returning a domain `ReportValidationError`, and have the CLI map that to `CliError`. Pairs naturally with `reducer-state-machine-hardening` since both move §7.3 invariants from the CLI to the canonical core.

Source: `issues/node-cli-read/handoff.md` D3.

## Resolution

`validate_report_payload` + its sub-validators now live in
`crates/octl-core/src/report.rs`, returning a domain-typed
`ReportValidationError` (thiserror). `octl_core::report` is reachable
without depending on `octl-cli`, so the supervisor can validate child
reports with the same rules (design.md §7.3 step 3).

- The CLI (`crates/octl-cli/src/node/report.rs`) delegates to the core
  validator and maps `ReportValidationError` → `CliError`
  (`schema_violation`) at the boundary via `map_report_validation_error`,
  preserving the original `code`/`message`/`expected` hints. No CLI
  behavior change — existing `node report` / `event create` integration
  tests pass unchanged.
- Faithful move (no tightening). A 4-model `/llm-review` confirmed no
  behavioral divergence; triage in
  `history/assessment-report-validator-into-core.{json,md}`.
- Applied review fixes: centralized the accepted-`Kind` wire names in
  `Kind::WIRE_NAMES` (schema.rs) with a serde round-trip drift guard so the
  unknown-kind `expected` hint can't drift from the enum; restored the
  faithful `success.as_bool().expect(...)`; tightened core tests
  (exact `expected()` content + extra branch coverage). Core now has 4
  valid + 15 invalid payload tests.

Boundary with F7 (`reducer-state-machine-hardening`) respected: the
reducer's call pattern and `crates/octl-core/src/reducer.rs` were not
touched.
