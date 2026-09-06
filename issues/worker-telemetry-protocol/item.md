---
created: 2026-08-22
updated: 2026-08-23
type: feature
reporter: jari
status: done
priority: normal
related: ['@supervisor-stall-detection', '@worker-wedged-one', '@end-end-stint', '@add-configurable-agent']
lane: lifecycle
lane_seq: 30
collision: [run-create]
closed: 2026-08-23
closed_by: pi
commits:
- hash: b414f40
  summary: Phase 1 worker telemetry design and feasibility
---

# Harness-neutral worker telemetry protocol with a pi.dev adapter

## Description

## Goal

Design the smallest reliable worker-status protocol for autonomous taskfleet runs. Replace process/activity heuristics with facts explicitly emitted by a harness adapter. The first adapter is a separately packaged pi.dev extension; taskfleet remains harness-neutral and owns the durable run state.

The protocol reports the latest lifecycle state explicitly told by a harness adapter and whether that adapter has refreshed recently. It does not diagnose progress or distinguish a healthy long-running operation from a wedged one; adding such inference would recreate the removed stall heuristics. Silence must not imply success, failure, retry, or teardown authority.

## Product shape

- A pi.dev extension observes documented lifecycle events such as agent start/settled, turn boundaries, and tool execution start/update/end.
- While the pi worker session is alive, the adapter sends a bounded periodic lease/keepalive plus a small explicit state to a public taskfleet CLI/JSON endpoint.
- The state may distinguish at least agent-active, tool-running, idle/settled, and adapter-shutdown; design must determine the minimal vocabulary and whether tool name plus elapsed time is safe and useful.
- Missing or expired telemetry means only `telemetry unavailable/stale` and requests attention. It must never imply merge success, terminal failure, or authorize teardown.
- Existing typed terminal outcomes and `run merge` remain the only success truth. Existing worker process exit recording remains the terminal crash backstop.
- Claude workers do not pretend to support this contract. Unless a future Claude adapter exists, Claude-backed worktrees are restricted to explicit interactive use rather than autonomous supervision.

## Boundary

The pi.dev adapter belongs outside this repository as a small installable pi package. It may call a documented taskfleet command and use public pi.dev extension events, but taskfleet must not import pi packages, access an extension manager/EventBus, assume pi process IDs or logs, or make session-scoped extension state canonical.

This is distinct from the rejected custom background-job manager. It reports the state of the current worker session; it does not own durable jobs or replace the neutral runner contract.

## Design questions

1. What run/node/attempt identity and local authorization does the adapter receive, and how is stale telemetry from a prior retry rejected?
2. What is the smallest state vocabulary that provides real diagnostic value without recreating activity heuristics?
3. What heartbeat interval, expiry, clock semantics, write-amplification bound, and restart behavior are safe?
4. Should heartbeats be append-only events, a separately replaceable lease projection, or another bounded durable representation?
5. How does `run show` / `run wait` distinguish active telemetry, stale telemetry, no adapter, and an explicitly settled agent?
6. How are the current tool and elapsed time reported without leaking command arguments, secrets, or unbounded output?
7. How is pi capability advertised and enforced so an unsupported harness cannot be launched autonomously by mistake?
8. Which parts belong to taskfleet, the external pi package, and the end-to-end stint lifecycle?

## Phase 1 — design and feasibility only

- Validate the proposal against pi.dev's documented extension lifecycle and a minimal throwaway adapter prototype.
- Map the earlier removed stall heuristics and state explicitly which failure modes must not return.
- Define the public CLI/JSON contract, durable state semantics, expiry behavior, and security/privacy limits.
- Compare at least a periodic heartbeat, event-only status transitions, and a hybrid lease model.
- Decide the external repository/package ownership and the manual/no-adapter behavior.
- Record the design and split implementation into independently reviewable issues.
- Stop for human review before implementing the production protocol or extension.

## Acceptance Criteria

- [x] The design is based on told adapter facts, not inferred process activity.
- [x] Silence can surface missing telemetry but cannot terminalize a run or delete work.
- [x] The pi adapter can report tool lifecycle and keepalive through public APIs without blocking the agent turn.
- [x] Unsupported harnesses are represented honestly; Claude autonomous runs are not silently treated as telemetry-capable.
- [x] The contract is harness-neutral and usable by a future adapter without pi-specific fields.
- [x] The external package boundary and installation/trust model are explicit.
- [x] The design includes failure injection for adapter crash, pi crash, taskfleet unavailability, delayed/duplicate heartbeat, stale attempt, long healthy tool execution, and clean agent settlement.

## Related work

- `@supervisor-stall-detection` (superseded heuristic proposal)
- `@worker-wedged-one` (observed long-command incident)
- `@end-end-stint` (durable stint/checkpoint lifecycle)
- `@add-configurable-agent` (harness capability and autonomous/interactive selection)
- `@pi-background-jobs-extension` (obsolete; records the separate rejected background-job design)

## Resolution

### 2026-08-23T06:44:41Z · @pi

Phase 1 design and feasibility slice completed. Production work remains blocked on human review; five implementation candidates were filed untriaged for lane-or-close disposition.
