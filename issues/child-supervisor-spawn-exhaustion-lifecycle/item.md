---
created: 2026-07-26
updated: 2026-07-26
type: improvement
status: open
priority: normal
related: ['@child-supervisor-spawn-unconfirmed-no-retry']
---

# Child-supervisor spawn: propagate exhausted-retry to run lifecycle + unify spawn-state enum

## Description

Follow-up from child-supervisor-spawn-unconfirmed-no-retry (the Starting/Failed retry state machine landed there). Two deferred items surfaced by the 4-model review (history/review-child-supervisor-retry.md):

1. **Exhaustion has no lifecycle propagation.** After CHILD_SPAWN_MAX_ATTEMPTS boot attempts with no identity-verified pid, the child stays ChildSpawn::Failed with its tail open. The parent's no-worker guard (child_tails.is_empty()) and all_work_done then can never fire, so the parent polls forever and the child agent's node shows pending in the UI with no terminal state. NOTE: this is status-quo (the old pid-0 path also left the tail open forever) and is now reached only after 5 bounded retries — but it should be closed. On exhaustion, emit a durable event the reducer folds into a terminal child failure (fail the child run / synthesize a terminal node.report) so rollup can wind the parent down, and drop the entry from child_spawns. Care: a very late boot could still claim the pid; gate the terminalization so it does not fight a live child.

2. **Unify spawned_children + child_spawns into one Running|Starting|Failed enum.** Two disjoint maps with a convention-enforced invariant is a smell (Opus). Bigger blast radius: spawned_children is persisted (u32) and read by the no-worker guard + shutdown-signal union, so this is a serialization + multi-reader change. Defer until the lifecycle work above is in.

Lower-priority notes from the same review (accept-as-is unless they bite): blocking CHILD_DIR_WAIT (5s) on the retry path is a no-op in practice (dir already exists by retry time); TOCTOU pid-read-vs-fork is mitigated by the child's claim_pid_atomic flock; retry backoff has no jitter.
