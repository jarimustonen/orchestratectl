---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: fixed
priority: high
lane: taskfleet-rename
lane_seq: 88
collision: [repository-identity]
commits:
- hash: 5a098130bd5ae829e874439252aff050e4dedb2e
  summary: 'test: declare bare CI spawn and archive dependencies'
- hash: '2038372'
  summary: 'test: make native materialization hermetic'
closed: 2026-09-03
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

## Reopen Notes — 2026-09-03

_Add rationale for reopening here._

## Comments

### 2026-09-03T11:57:12Z · @orchestrator

Acceptance failed on exact-SHA main CI 33751749394 (bc8d06c): Ubuntu still reports no_tmux_session for exact-argv materialization; macOS reports no_tmux_session in spawn_all_kinds::each_kind_native_spawn_publishes_a_live_handshaken_node; nextest also flags shim_forwards_sigterm_and_records_the_childs_true_exit as leaky. Reopened for a complete bare-Linux/macOS materializing-test audit; R8 remains blocked.

### 2026-09-03T14:01:56Z · @agent

Completed the reopened portability audit. CI log review corrects the reopen summary: in run 33751749394 the exact-argv test passed on both Ubuntu and macOS; spawn_all_kinds failed with no_tmux_session on both OSes, and the SIGTERM test leaked on both. The platform-dependent behavior was ambient TMUX in equipped developer sessions versus bare CI, not an Ubuntu/macOS production divergence. NativeSpawnTools now strips TMUX centrally, every materialized create declares isolated public-CLI placement, and a negative test pins bare-context rejection. The LEAK was an orphaned sleep grandchild retaining nextest descriptors after the shell received forwarded SIGTERM; the test now execs the workload and owns panic cleanup in a private process group. No-fail-fast also exposed and fixed two test-only macOS concurrency races: a 10s readiness allowance and a partial capture-file read. Final macOS and bare-Linux full 1,111-test no-fail-fast runs passed with no LEAK markers; fmt, clippy, doctests, rustdoc, and version snapshots are green. No fixture worktree, tmux, run-root, supervisor, or child-process residue remains.
