---
created: 2026-09-03
updated: 2026-09-03
type: improvement
reporter: jari
status: done
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 87
collision: [repository-identity]
closed: 2026-09-03
commits:
- hash: f7193a1
  summary: 'feat: internalize native worktree materialization'
---

# Internalize Taskfleet worktree materialization

## Goal

Remove Taskfleet's undocumented production dependency on the Homebase-owned `~/.claude/skills/worktree/scripts/create.sh` before ADR 0002 R8 is retried.

## Context

All bundled worktree skills correctly invoke `taskfleet run create` and explicitly forbid direct `create.sh`/`workmux` use. However `crates/taskfleet/src/run/spawn.rs` still shells out to a personal Homebase symlink retained from the MVP design's explicitly deferred native Rust port. A clean machine without Homebase fails `run create`; pi 0.84.4 exposed the boundary when process-name guessing rejected native `pi`.

## Scope

- Internalize worktree/tmux/agent materialization in Taskfleet production code, preserving current run/event/supervisor contracts and partial-failure cleanup.
- It is acceptable to retain `workmux` as an explicit external CLI dependency; the private Homebase script is not acceptable.
- Replace agent-name process-tree guessing with a private per-attempt PID handshake written by Taskfleet's generated launcher immediately before `exec` of the exact recorded candidate. Validate PID, start identity, attempt/node/run ownership and liveness before publication.
- Preserve source branch, headless/parent session, layout, hooks, qualified tmux identity, prompt delivery, exact argv, telemetry env, retry and cleanup semantics.
- Remove the production default path to `~/.claude/skills/worktree/scripts/create.sh`. Any retained spawn test seam must be clearly test-only and cannot make a clean install depend on Homebase.
- Update dependency/help/doctor/docs/package contracts truthfully.

## Acceptance Criteria

- [x] `taskfleet run create --harness pi` succeeds in a disposable HOME where `.claude/skills/worktree/scripts` does not exist.
- [x] Native pi, Node-backed candidates, Claude-compatible recorded candidates, immediate exits, stale/forged handshakes, PID reuse, timeout, wrong attempt and partial workmux/tmux/git failures are covered.
- [x] No process executable-name inference remains in Taskfleet's production spawn path.
- [x] Existing state, stdout/JSON, exit, self-exec and teardown behavior remains compatible.
- [x] Full Rust, stripped-PATH, snapshot, package and integration gates pass.
- [x] No global install, user-state migration, release, tag, repository rename or tap activation occurs.
## Implementation

Completed in run `01m1k30q6w8vw2f0824shbh224`. Taskfleet now owns native git/workmux/tmux materialization, exact generated candidate launchers, a private run/node/attempt-bound PID handshake with precise process birth identity, atomic publication, retry materialization, and rollback/reclaim behavior. Production no longer resolves or invokes the Homebase create script and no executable-name or descendant-process inference remains.

Validation includes all spawn kinds, native pi in a disposable HOME, Node-backed and Claude-compatible candidates, exact hostile argv, immediate and missing exits, stale/forged/wrong-attempt/PID-identity handshakes, workmux and interrupted failures, supervisor boot failure, the full native spawn→supervise→merge→teardown round trip, snapshots, package inspection, and the repository green gate. Review artifacts are in ignored `history/2026-09-04-review-taskfleet-native-materialization*.md` and `history/assessment-2026-09-04-taskfleet-native-materialization.*`.

## Resolution

### 2026-09-03T09:42:01Z · @issuectl

Native materialization, exact launcher handshake, rollback/retry semantics, and all acceptance gates completed.
