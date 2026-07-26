---
created: 2026-07-26
updated: 2026-07-26
type: improvement
reporter: jari
status: in-progress
priority: normal
---

# Bounded auto-retry on agent-died for autonomous single-node workers (intermittent deaths recover without a human)

_Source: supervise: agent-died recovery_

## Description

Intermittent worker-process deaths (agent-died) are now confirmed transient, not deterministic: the pipeline-tiered-triage task died at ~13 min twice, then a third identical spawn ran ~54 min and landed cleanly (run 01kyes8wmg, via explicit-merge). See @worker-process-hang and @agent-death-strands-recoverable-work. The supervisor currently synthesizes a terminal agent-died failed report on the FIRST death and stops; recovery is fully manual (the orchestrator re-spawns by hand). That manual re-spawn cost 3 attempts on one task this session.

## Ask
Give the top-level single-node autonomous worker a BOUNDED auto-retry on agent-died, symmetric with the child-supervisor bounded retry just landed (@child-supervisor-spawn-unconfirmed-no-retry). On a synthesized agent-died with NO recoverable committed work (branch == base, no commits ahead of source), the supervisor re-spawns the agent up to a small max-attempts with backoff, before finally terminalizing as failed. Compose with @agent-death-strands-recoverable-work: if the dead agent DID leave clean committed work, prefer salvage over a fresh retry (a retry would start from base and lose nothing, but salvage lands the existing work — decide the precedence).

## Constraints / notes
- Bounded only (max attempts + backoff); never an unbounded respawn loop. Reuse the child-supervisor retry policy shape if it generalizes.
- A retry must start from a clean worktree at the run's source branch (the dead agent left the worktree at base with no commits in the empty-handed case).
- Interactive kind: do NOT auto-retry (a human is driving); this is for autonomous single-node kinds.
- Emit durable events for each retry attempt so the history is auditable (attempt N, reason).
- Hot files: crates/octl-cli/src/supervise/* (mod.rs agent-died synthesis / reconcile path), possibly events.rs/reducer.rs for a retry event. Preserve every state-integrity invariant and the teardown preservation gates.

## Acceptance
- An autonomous worker that dies agent-died with no committed work is re-spawned up to max-attempts before the run is terminalized failed; each attempt is a durable event.
- No auto-retry for interactive kind, or when recoverable committed work exists (salvage path wins).
- Bounded: a persistently-dying agent terminalizes failed after max-attempts, never loops forever.
- Regression: a healthy run (no death) is unaffected; existing agent-died teardown/preservation gates intact.
