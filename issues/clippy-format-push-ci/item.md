---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: fixed
priority: high
closed: 2026-08-17
---

# Clippy format push warnings break CI

_Source: crates/taskfleet-cli/tests/skill.rs_

## Description

Commit 6c5f03b appended format! output to existing Strings in skill integration tests. Workspace clippy enables clippy::pedantic and CI runs with -D warnings, so both format_push_string warnings make main red. Replace both allocations with direct formatted writes to String and verify the full CI gate.

## Resolution

### 2026-08-17T19:29:13Z · @issuectl

Replaced both format_push_string call sites with infallible writeln! writes to String. Verified cargo fmt, CI-exact clippy with -D warnings, the workspace test suite, and rustdoc with warnings denied.
