---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: fixed
priority: high
related: ['@taskfleet-integrated-validation']
lane: taskfleet-rename
lane_seq: 88
collision: [repository-identity]
closed: 2026-09-03
commits:
- hash: 5a098130bd5ae829e874439252aff050e4dedb2e
  summary: 'test: declare bare CI spawn and archive dependencies'
---

# Restore bare-CI portability before Taskfleet R8

## Observed occurrence

Exact-SHA main CI run `33746392679` at `3df561df56cd8af3842007ac849325529b1db2eb` is red after the native materialization and first publish-fixture portability fixes.

1. Linux and macOS nextest fail `taskfleet::run::materialized_create_routes_through_the_recorded_exact_argv`: the test invokes production `run create` without `--headless` or `--tmux-session`. It passed in worker tmux sessions by inheriting ambient `TMUX`, but correctly fails on bare CI with `no_tmux_session`.
2. `version-snapshots` reaches archive creation in `scripts/test-publish-crates.sh`, then its stripped PATH lacks `gzip`; GNU tar shells out to gzip and fails with status 127. macOS bsdtar behavior hid the undeclared prerequisite.

## Goal

Make both tests declare and isolate every required execution context/tool so the same exact behavior runs under bare Linux/macOS CI and equipped developer tmux sessions.

## Acceptance criteria

- Exact-argv materialization test explicitly removes ambient `TMUX`, supplies a stubbed/isolated tmux session through the supported CLI surface, and still proves exact recorded argv + handshake semantics.
- Publish fixture declares/provides the exact archive compression prerequisite under stripped PATH without exposing unrelated host tools or weakening registry assertions.
- Regression tests fail under the old behavior on both an ambient tmux developer shell and bare/stripped CI-like environment.
- Full Rust gate and every version-snapshot/release script pass on macOS and an available Linux/container equivalent.
- Zero worktree/tmux/run residue outside declared sandboxes.
- No publication, install, tag, repository rename or tap mutation.

## Resolution

### 2026-09-03T11:47:21Z · @issuectl

Made the native exact-argv test explicitly TMUX-free with its private named session, and declared gzip in the stripped publish fixture. Focused macOS/Linux checks, release protocols, and the full Rust green gate pass.
