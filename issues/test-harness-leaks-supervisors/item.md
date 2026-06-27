---
created: 2026-06-27
updated: 2026-06-27
type: bug
status: done
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

## Resolution (2026-06-27)

Two-sided fix; multi-model `/llm-review` (Gemini 3.1, GPT-5.5, Opus 4.7, DeepSeek v4) run on the
diff — assessment in `history/review-test-harness-leaks-supervisors.md`.

**Product hardening (`crates/octl-cli/src/supervise/mod.rs`).** Orphan defense checked at the TOP
of the main loop (before any side-effecting work): if `manifest.json` is absent (via `try_exists`,
so a transient stat error doesn't count) for `SELF_TERMINATE_TICKS = 3` consecutive 1s polls (~3s),
the supervisor self-terminates — exit 0, emitting `supervisor.self-terminated` when the events log
survives. Manifest writes are atomic (tempfile+rename) so manifest.json is never transiently absent
from a legitimate rewrite.

Critical review finding (all 4 reviewers): `state::save` and `append_and_apply_event` write through
`create_dir_all`, so the naïve version *resurrected* the deleted run dir every tick. Fixed by
(a) hoisting the check above all IO and (b) adding `write_atomic_no_create` /
`write_json_atomic_no_create` to octl-core and switching `state::save` to the non-creating variant —
a vanished run dir now makes the per-tick save fail harmlessly instead of resurrecting the dir.

**Test fixes.** `tests/spawn_all_kinds.rs` gained a Drop-guard `SupervisorReaper` (captures the
supervisor PID from the `run create` response, SIGTERM → 2s grace → SIGKILL, errno-aware so it never
signals a recycled/foreign PID), replacing the racy `run cancel` cleanup. The `OCTL_TEST_SKIP_MATERIALIZE`
suites never reached supervisor spawn (early return in create.rs), so only these two `spawn_all_kinds`
tests actually leaked. New `supervise_gates.rs` tests: `self_terminate_when_run_dir_vanishes`
(manifest-only removal → exit 0 + `supervisor.self-terminated` event) and
`self_terminate_when_whole_run_dir_removed` (whole dir removed → exit 0 + asserts no resurrection).

**Verified:** build + `cargo test --release --workspace` + clippy + fmt clean; 0 leaked supervisors
from this worktree's binary after the full suite; whole-dir test deterministic over 12 consecutive runs.

Deferred to `supervisor-child-detach-reap`: cascade-SIGTERM tracked child supervisors on self-terminate
(today self-healing — each child's dir vanishes simultaneously and it self-terminates independently).
