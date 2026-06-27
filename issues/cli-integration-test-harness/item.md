---
created: 2026-06-12
updated: 2026-06-27
type: task
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
---

# Integration test harness with insta envelope snapshots

## Description

Add tests/ harness using assert_cmd + predicates + insta to lock the success-envelope and error-envelope shapes via snapshots. Without this, every subcommand contributor invents their own test scheme and envelope drift goes undetected. insta is already in dev-deps but unused. Surfaced by cargo-scaffolding review.
