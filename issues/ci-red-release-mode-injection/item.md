---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: fixed
priority: high
lane: lifecycle
lane_seq: 1
commits:
- hash: 2d0122c
  summary: start release injection CI fix
- hash: 836ba7c
  summary: skip debug-hook cases in release builds
closed: 2026-08-17
---

# creation_reliability test red on CI: release build ignores debug-only injection hooks

## Description

## Symptom

Main CI red on e5f0bb6: `retry_repairs_published_child_missing_parent_edge` in `crates/octl-cli/tests/creation_reliability.rs:350` fails on BOTH ubuntu and macos with `assertion failed: !interrupted.status.success()`.

## Root cause

CI runs `cargo test --locked --release --workspace`. The test injects `OCTL_TEST_FAIL_AFTER_PUBLISH=1` / `OCTL_TEST_SKIP_MATERIALIZE=1`, but the hook in `crates/octl-cli/src/run/create.rs:680` is gated on `cfg!(debug_assertions)` — deliberately, so a production binary never honors a test kill switch. In a release build the injection is a no-op, the interrupted create exits 0, and the test's expectation fails. The local green gate runs debug and is structurally blind to this.

## Expected

The debug-only gating of the injection hooks stays (production binaries must not honor test kill switches). The tests that depend on injection must not run against a release binary — e.g. guard those test fns with `#[cfg(debug_assertions)]` (the integration-test crate compiles with the same profile as the binary under test) or an equivalent explicit skip, so `cargo test --release --workspace` is green while debug runs keep full coverage. Verify locally with `cargo test --release -p orchestratectl --test creation_reliability`.

## Context

Introduced by `create-idempotency-lease-recovery` (5a81ff3/d179a27) this round. Blocks the v0.3.0 tag (CI-green-on-tagged-commit is a release precondition).
