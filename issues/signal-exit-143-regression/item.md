---
created: 2026-08-13
updated: 2026-08-13
type: bug
status: open
priority: high
related: ['@supervise-gates-signal-wait-hardening', '@supervise-gate-test-flake']
---

# CI red on main: signal_exit_codes_and_payload — SIGTERM exits 512 not 143

## Description

`cargo test --locked --release --workspace` fails on `main` (run 31611457752, commit `72a545e`, ubuntu-latest): the supervise-gates signal test regressed.

## Symptom (real log lines)

```
failures:
thread 'signal_exit_codes_and_payload' (16665) panicked at crates/octl-cli/tests/supervise_gates.rs:1191:9:
assertion `left == right` failed: SIGTERM must exit 143, got ExitStatus(unix_wait_status(512))
test result: FAILED. 24 passed; 1 failed; 0 ignored
##[error]Process completed with exit code 101.
```

`unix_wait_status(512)` = exit code 2 (512 >> 8), i.e. the supervised child terminated with a **normal exit 2** rather than being killed by SIGTERM (which would give 143). So the assertion at `supervise_gates.rs:1191` that SIGTERM yields 143 no longer holds — the process is exiting on its own (or a teardown race) before/instead of the signal path.

## Why this matters as a regression

This exact test area was hardened and marked done in `@supervise-gates-signal-wait-hardening` and `@supervise-gate-test-flake`. The failure recurring means that hardening did not fully hold — either a genuine signal-handling regression on `main`, or the test is still racy under `--release` load. Reproduce locally with `cargo test --locked --release -p octl-cli signal_exit_codes_and_payload` before deciding fix vs. re-harden.

## Fix direction

Determine whether the child is legitimately exiting 2 (real bug in the SIGTERM path) or the test observes the wrong process/exit under a teardown race (flake). If flake, bound the wait and assert on the signal path deterministically per the prior hardening issues; if real, fix the supervisor's SIGTERM propagation.
