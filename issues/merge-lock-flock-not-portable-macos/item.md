---
created: 2026-08-10
updated: 2026-08-10
type: bug
reporter: jari
status: fixed
priority: high
commits:
- hash: 2e8bd5e
  summary: portable mkdir merge lock replaces flock
- hash: a3dbae7
  summary: harden lock per llm-review (remove racy stale-reclaim, classify mkdir errors, cap timeout)
closed: 2026-08-10
---

# merge.sh depends on flock, absent on stock macOS — merge lock silently broken on primary platform

_Source: crates/taskfleet-cli skills merge.sh_

## Description

merge.sh uses flock (line ~138) to serialize merges on the target branch. flock ships with util-linux and is NOT present on stock macOS (only via homebrew keg-only util-linux at /opt/homebrew/bin/flock). On a macOS runner without it, merge.sh emits 'flock: command not found' and exits 75, which the CLI misreads as merge_in_progress. This has kept test (macos-latest) red for days (4 run_merge.rs tests: concurrent_self_merge_waits_then_succeeds, concurrent_self_merge_serializes_instead_of_false_dirty, downstream_exit_75_is_not_merge_in_progress, genuine_dirty_target_still_blocks) and — more importantly — means the worktree-merge lock is silently non-functional for stock-Mac users who brew-install the tool (macOS is the PRIMARY platform, shipping aarch64-mac binaries). Local dev only passes because homebrew put flock on PATH. Fix: make the merge lock portable (e.g. an atomic mkdir-based mutex in merge.sh preserving the 600s timeout + exit-75 semantics, or move the lock into the Rust side via taskfleet-core's lock layer) so it works without flock. Preserve all merge-lock semantics the run_merge.rs tests encode (exit 75 = merge_in_progress serialization retry vs other failures = merge_failed). Touches a correctness-sensitive path — needs /llm-review. NOTE: ci-red-main-deny-docs was closed assuming the macos failure was broken doctests; that diagnosis was wrong — this flock gap is the real cause.
