---
created: 2026-07-26
updated: 2026-08-13
type: improvement
reporter: jari
status: obsolete
priority: normal
related: ['@autoretry-agent-died-worker']
closed: 2026-08-13
closed_by: adr-decision-2
---

# Crash-consistency hardening for agent-died auto-retry (durable retry-pending marker + CAS branch deletion)

_Source: supervise: agent-died auto-retry_

## Description

Follow-up from autoretry-agent-died-worker's /llm-review (history/review-autoretry-agent-died-worker.md). The landed bounded auto-retry is correct/safe for the common case (retry-vs-salvage holds; bound is durable via Node.retry_attempts; teardown re-verifies empty-handedness; a fresh spawn is torn down on every abort path). Residual rare-race hardening deferred to keep the initial change bounded:

1. Durable 'retry-pending' marker before create.sh. In-memory RetryPark + durable Node.retry_attempts bound retries today; a supervisor crash BETWEEN a successful create.sh and the node.retry append leaves an orphan -rN worktree (empty) and, on restart, the deterministic -rN branch name can collide (create.sh fails -> spawn_failures budget -> terminalize). Bounded + no data loss, but a durable pre-spawn phase marker (node.retry.scheduled/attached) would make the re-spawn fully crash-consistent and restart-resumable. PRIORITIZE this - it subsumes most others.

2. CAS branch deletion. cleanup_node's -d backstop + Dead-only gate + source-relative preserve make teardown safe in practice, but the run lock does not serialize git ref updates. git update-ref -d refs/heads/<b> <expected_oid> (CAS against the exact tip proven empty) closes the theoretical window where a subprocess of a dead agent advances the ref between the empty-handed proof and deletion.

3. agent_pid_start_time capture on respawn. RespawnOutcome/node.created do not record PID start time -> weaker pid-recycle detection for respawned (and freshly created) agents. Pre-existing gap shared with run create.

4. spawn_failures durability across restart. In-memory counter; a supervisor that repeatedly crashes AND has broken create.sh could re-attempt the spawn budget each restart. Persist it (or the retry phase) alongside retry_attempts.

None are data-loss or unbounded-loop in the landed design; crash-consistency/robustness refinements only.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

The agent-died auto-retry synthesizer is deleted; A1/A6 -> non-zero exit is a told 'failed' (preserve branch), no retry loop — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
