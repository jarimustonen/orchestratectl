---
created: 2026-06-27
updated: 2026-06-27
type: bug
status: open
priority: normal
epic: orchestratectl-mvp
---

# Integration tests leak detached supervisor processes

## Description

Observed while merging core-path-traversal-id-validation: hundreds of orphaned `orchestratectl supervise` processes had accumulated across parallel worktree sessions. ~257 were force-killed to recover.

## Symptom

Long-lived `orchestratectl supervise <ulid>` processes keep running after the test that spawned them finishes. They poll their run dir forever (TAIL_TICK 500ms / WATCHDOG_TICK 1s); because the test's run dir is a TempDir that gets deleted on test teardown, they end up polling a vanished directory indefinitely. They also fork child supervisors, so the count multiplies. Across many parallel /worktree sessions each running `cargo test`, this reaches hundreds of processes — wasting CPU and file descriptors, and surviving even after the worktree (and its target/ binary) is removed by /worktree-merge.

## Root cause (hypothesis)

Integration tests that spawn a *real* supervisor (not `--once` / not signal-terminated) — primarily `tests/spawn_all_kinds.rs::each_kind_spawns_and_emits_node_created` and possibly the `run create`/`run reattach` real-spawn paths — start a detached process via create.sh / supervisor_spawn and never reap it. TempDir's Drop removes the on-disk state but does not signal the supervisor.

## Suggested fixes (pick per case)

- Tests that spawn real supervisors should kill them in teardown (capture the supervisor PID from the run's supervisor.pid / the create response and SIGTERM it, or wrap in a guard that kills on drop).
- Consider `--once` / `--max-iter` for tests that don't actually need a long-lived supervisor.
- Product hardening: a supervisor whose run dir disappears (manifest/events gone) should self-terminate rather than poll a deleted directory forever — defense against orphaning in production too.
- Optional: a `make`/CI helper that reaps stray `orchestratectl supervise` processes after the test suite.

## Repro

Run `cargo test --all` (or just `-p octl-cli --test spawn_all_kinds`) a few times, then `pgrep -fl 'orchestratectl supervise'` — orphans remain after the run completes.
