---
created: 2026-07-25
updated: 2026-07-26
type: bug
status: fixed
priority: normal
related: ['@supervisor-spawn-fails-silently-at-run-create']
closed: 2026-07-26
---

# Child-supervisor spawn records pid 0 as success and never retries a failed child boot

## Description

`spawn_child_supervisor` (crates/octl-cli/src/supervise/mod.rs) treats a successful
double-fork as a successful child-supervisor start: it does a single non-blocking
`read_live_recorded_pid(...).unwrap_or(0)` and the caller inserts the child into
`state.spawned_children` even when the pid is 0. A later tick skips re-spawning
(`if state.spawned_children.contains_key(...) { continue; }`), so a child supervisor
that dies before writing its pid file is NEVER retried. Because the child run already
has node_count >= 1, the new top-level no-worker guard (issue
supervisor-spawn-fails-silently-at-run-create) does not catch it either — the child
agent can sit `pending` indefinitely.

Raised by GPT-5.6 in the creation-path reliability review (finding F11,
history/review-creation-path-reliability.md). Pre-existing behavior; NOT introduced by
the creation-path fixes, so it was left for its own change.

**Fix direction:** represent child-supervisor startup as a state machine
(Starting{since} / Running{pid} / Failed{attempts, retry_at}) instead of inserting
pid 0 as success: poll `read_live_recorded_pid` without blocking, promote to Running
only on an identity-verified pid, and after a deadline emit `child.spawn_failed` and
retry with a bounded policy. At minimum, do not insert into `spawned_children` when the
pid is 0.
