---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: open
priority: normal
---

# Integration tests leak supervisor processes on teardown

## Description

Symptom: heavy cargo test cycles across multiple worktrees leave N>>1 orphaned `orchestratectl supervise <run-id>` processes parented to init.

2026-06-28 (haukinen): 180 orphans from 6 since-deleted worktrees - id-canonical-form-validation (91), help-json-structured (36), supervise-gate-test-flake (27), log-guard-flush-on-process-exit (17), core-append-and-apply-api (9), plus an unattributable batch (14). Each process 0% CPU, no open writable FDs; 177/180 reference a run dir that no longer exists under ~/.orchestratectl/runs/.

Root cause hypothesis: integration tests in crates/octl-cli/tests/supervise_gates.rs (and likely others) spawn real supervisor processes via run create's production path. Test teardown does not reliably reap them - panic/kill/early-exit from the harness leaves the supervisor parented to init. When the worktree is later merged and deleted, the binary's inode is held alive by the orphan and it keeps idle-polling.

Fix direction: (1) Drop impl on the test fixture that kills every supervisor PID it spawned (track them in the fixture). (2) Belt-and-braces: process-group-level kill on test exit covering any descendant of the test PID. (3) Audit which other test files spawn real supervisors (grep -r 'supervise' crates/*/tests/).

Workaround: pkill -f 'orchestratectl__worktrees/.*supervise' (safe - production supervisors live under ~/.cargo/bin/orchestratectl, not __worktrees/).
