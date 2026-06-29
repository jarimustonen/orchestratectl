---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: open
priority: normal
---

# Flaky test: self_terminate_when_whole_run_dir_removed (timing-sensitive)

## Description

`tests/supervise_gates.rs::self_terminate_when_whole_run_dir_removed` fails intermittently under `cargo test --workspace` but passes reliably when run in isolation:

```
$ cargo test --workspace 2>&1 | grep self_terminate_when
test self_terminate_when_whole_run_dir_removed ... FAILED   # sometimes

$ cargo test --workspace self_terminate_when_whole_run_dir_removed 2>&1 | grep ^test
test self_terminate_when_whole_run_dir_removed ... ok      # always
```

Observed twice during the 2026-06-29 pre-publication session. The test exercises the supervisor's self-terminate path when its own run directory vanishes mid-poll — a deletion-versus-poll race against the filesystem. Under load (parallel `cargo test --workspace` runs other supervise tests concurrently) the timing window can shift enough that the test reads a transient state and fails its terminal assertion.

Not blocking v0.1.0 — CI re-runs and isolated invocations both pass. But on a fresh CI machine under sustained load this is the test most likely to fail spuriously and waste a re-run.

## Likely fix

Either:
1. Mark `#[serial]` (via `serial_test`) so supervise tests don't race each other on the global filesystem.
2. Loosen the assertion to a "settles within N polls" loop rather than a single-tick read.
3. Add a setup barrier so the deletion only happens after the supervisor has done at least one observable poll.

Prefer (1) — simplest, doesn't change the contract under test.
