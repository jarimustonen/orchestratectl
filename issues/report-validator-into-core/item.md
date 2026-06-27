---
created: 2026-06-12
updated: 2026-06-27
type: feature
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Move §7.3 report validator into octl-core

## Description


`validate_report_payload` lives in `crates/octl-cli/src/node/report.rs` and returns `CliError` directly. When `supervisor-process` lands and needs to validate child reports before consuming them (§7.3 step 3), it cannot call the same function — it would have to copy it or pull the CLI as a dependency.

Move the validator and its sub-validators to `octl_core::report` returning a domain `ReportValidationError`, and have the CLI map that to `CliError`. Pairs naturally with `reducer-state-machine-hardening` since both move §7.3 invariants from the CLI to the canonical core.

Source: `issues/node-cli-read/handoff.md` D3.
