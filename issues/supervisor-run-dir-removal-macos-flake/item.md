---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: open
priority: high
related: ['@taskfleet-zero-legacy-identity']
lane: taskfleet-rename
lane_seq: 145
collision: [crates/taskfleet/tests/supervise_gates.rs]
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
