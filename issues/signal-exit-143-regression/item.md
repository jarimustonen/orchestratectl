---
created: 2026-08-13
updated: 2026-08-16
type: bug
status: fixed
priority: high
related: ['@supervise-gates-signal-wait-hardening', '@supervise-gate-test-flake']
closed: 2026-08-16
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

## Resolution (2026-08-13) — real §7.8 exit-code gap, fixed in the supervisor

It was BOTH: the test's readiness gate (pid-file existence) legitimately fires
inside the boot window, AND the supervisor genuinely violated §7.8 there.

Root cause: `unix_wait_status(512)` = exit 2 = `ExitKind::System`. A SIGTERM
delivered after the pid-file claim in `boot_supervisor` but before dispatch's
signal-check took the `terminated_during_boot` branch → `return
CliError::system(...)` → exit 2, bypassing the §7.8 clean-shutdown that emits
`supervisor.exited{reason:"signal"}` and `process::exit(143)`. The window is
sub-millisecond locally but widens under `--release` CPU load (the `supervisor.started`
flock+fsync + tail seeding run inside it), so it only bit on loaded CI.

Fix (`crates/octl-cli/src/supervise/mod.rs`): a dedicated **boot-signal
short-circuit** right after the boot destructure — on `SIGNAL_RECEIVED != 0` it
emits `supervisor.exited{reason:"signal"}`, removes the pid file, reports the
readiness error to the parent (AFTER the durable cleanup, so no teardown race),
and exits 130/143 via a new shared `finish_signal_exit` (also used by the loop
epilogue, so the two exit paths cannot drift). The loop-setup side effects never
run for a supervisor that is only going to shut down.

Regression guard: `signal_during_boot_exits_143` (parameterized over SIGTERM→143
and SIGINT→130) uses a bounded `OCTL_TEST_SLOW_BOOT` signal-barrier that holds
boot until the signal is provably observed in the boot window — deterministic,
and it failed (`exit 2`) against the pre-fix code. Reviewed via `/llm-review`
(4 models) + inline `/assess-findings`; see
`history/review-signal-exit-143-regression.md`. Green gate + integrated
`cargo test --workspace` clean.
