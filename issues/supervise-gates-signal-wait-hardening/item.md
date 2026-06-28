---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
related: ['@supervise-gate-test-flake']
labels: [test, review-spinoff]
closed: 2026-06-28
commits:
- hash: 1dea693
  summary: bounded signal wait
---

# supervise_gates signal test: bound child.wait() and check libc::kill return

## Description

Spin-off from supervise-gate-test-flake review (poll_until). In signal_exit_codes_and_payload (crates/octl-cli/tests/supervise_gates.rs): (1) `child.wait()` after sending the signal is unbounded — if the signal is dropped or the handler hangs, the test hangs indefinitely; poll `try_wait` with a deadline and kill on failure. (2) The `libc::kill` return value is ignored, hiding ESRCH/EPERM (which would then manifest as the hang above); assert rc==0 with last_os_error. Optionally strengthen the pid readiness check from existence-only to parsing the pid and confirming liveness. Pre-existing; flagged by GPT-5.5, out of scope for the pure de-flake.
