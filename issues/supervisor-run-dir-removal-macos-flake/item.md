---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: fixed
priority: high
related: ['@taskfleet-zero-legacy-identity']
lane: taskfleet-rename
lane_seq: 145
collision: [crates/taskfleet/tests/supervise_gates.rs]
closed: 2026-09-06
commits:
- hash: 2def0c4
  summary: synchronize supervisor removal tests on readiness
---

# Supervisor run-directory removal flakes on hosted macOS

## Problem

Exact-main CI run `34039201342` for clean Taskfleet identity commit `36de97abb7f5816a805356b17f2142f37b36de72` failed only in hosted macOS nextest. `taskfleet::supervise_gates::self_terminate_when_run_dir_vanishes` panicked at `crates/taskfleet/tests/supervise_gates.rs:1704`: supervisor did not self-terminate within 10 seconds after run-directory removal. The concurrently executed sibling `self_terminate_when_whole_run_dir_removed` passed in 13.7 seconds. Ubuntu, self-hosted macOS, MSRV, clippy, docs, snapshots, and release-topology jobs passed.

## Required work

Reproduce under macOS contention and determine whether the failure is a production liveness defect, shared-resource collision, or an unrealistically fixed test timeout. Fix the underlying synchronization/test-isolation issue rather than merely inflating sleep. Keep process ownership exact and ensure all spawned processes and temporary roots are reaped on success and failure. Add deterministic regression coverage and exercise the focused pair concurrently/repeatedly before the full gate.

Maintain the zero-legacy identity invariant: `./scripts/check-canonical-identity.sh` must remain green. Do not release, install, migrate state, deploy, or edit external repositories.

## Acceptance Criteria

- [ ] Focused run-directory-removal supervisor tests pass repeatedly and under concurrent nextest execution on macOS.
- [ ] No test process/root/tmux residue remains.
- [ ] Full repository green gate passes.
- [ ] Exact-main hosted macOS CI is green.

## Resolution

### 2026-09-06T15:22:34Z · @issuectl

Fixed the hosted-macOS timing race by waiting for the production readiness-pipe frame, verifying it names the exact direct child, and guarding that child with unconditional kill/reap cleanup. The former PID-file poll could remove manifest.json after PID claim but before supervisor.started projection application restored it. The sibling whole-directory test masked the same race with its removal retry. The tests now run concurrently; 12 concurrent repeated pairs plus 4 slow-boot contention pairs passed, followed by the exact full green gate and canonical identity check.
