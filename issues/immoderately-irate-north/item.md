---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: open
priority: normal
---

# watchdog snapshot invocation-count test is a parallel-execution flake

## Description

`supervise::watchdog::tests::snapshot_is_one_invocation_per_socket_regardless_of_node_count`
(`crates/octl-cli/src/supervise/watchdog.rs`) FAILS intermittently in the full parallel
`cargo test --workspace` run (`assertion left == right failed … left: 2, right: 1` —
`invocation_count` saw a 2nd fake-tmux invocation), but PASSES in isolation, single-threaded,
within its own module, and on retry.

Classic test-isolation flake: the test holds `test_env::lock()` and counts fake-tmux
invocations written into its own TempDir. A count of 2 means another test that shells out to
tmux (reads `TMUX_BIN`) **without** holding `test_env::lock()` appended to this test's counter
file while it held the lock — the guard only serialises tests that also take it.

Caught by the integrated gate on 2026-08-06 (round: cancel-terminal / muddled-caption stall /
pipeline-prov-refs — **none** touched `watchdog.rs`, so this is pre-existing, not
round-introduced). Green on the retry run; all round work verified landed on main.

**Fix direction:** audit watchdog (and any) tests for a tmux-invoking path that skips
`test_env::lock()`, or make `invocation_count` robust to cross-test pollution (e.g. a
per-invocation unique marker keyed to the test's dir/env, so a stray invocation from another
test can't be miscounted). Cousin of the prior `notify-test-toctou-flake`. Lane A
(`supervise/*`).

