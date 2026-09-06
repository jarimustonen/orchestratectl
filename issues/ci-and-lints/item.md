---
created: 2026-06-12
updated: 2026-06-27
type: task
status: done
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff, cargo-scaffolding-review]
closed: 2026-06-27
---

# CI workflow, rustfmt.toml, clippy.toml, deny.toml, workspace lints

## Description

Add .github/workflows/ci.yml (fmt --check, clippy -D warnings, test, cargo-deny), rustfmt.toml, clippy.toml, deny.toml, and [workspace.lints.clippy] pedantic config. Pin now before 30 subcommands arrive with inconsistent style. Surfaced by cargo-scaffolding review.
