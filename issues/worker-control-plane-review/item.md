---
created: 2026-08-22
updated: 2026-08-22
type: task
reporter: jari
status: open
priority: normal
related: ['@worker-telemetry-protocol', '@add-configurable-agent', '@end-end-stint']
lane: lifecycle
lane_seq: 50
collision: [run-create]
blocked_by: ['@worker-telemetry-protocol', '@add-configurable-agent']
---

# Review worker telemetry and agent profiles as one control plane

## Description

## Goal

Review the worker telemetry and configurable agent-profile designs as one coherent worker control plane before any production implementation is split or started.

This is the explicit human checkpoint in the sequence:

1. design `@worker-telemetry-protocol`;
2. revise the `@add-configurable-agent` design against that protocol;
3. synthesize both designs here and ask Jari whether the telemetry and profile model are acceptable as a whole;
4. only after explicit approval, file and schedule implementation slices.

## Questions to resolve together

- How does a profile declare whether its harness supports autonomous worker telemetry?
- Is `pi` with the approved adapter the only initially telemetry-capable autonomous harness?
- How is Claude restricted to interactive worktrees without breaking explicit interactive selection?
- Can fallback ever cross from a telemetry-capable autonomous candidate to one without telemetry? Default answer should fail closed.
- How do capability, data residency, interactivity, and telemetry support remain orthogonal and machine-checkable?
- How does `run create`, effective config inspection, run metadata, and `run show` explain the selected policy and its source?
- Which state belongs to the pi adapter, the harness-neutral orchestratectl protocol, and the end-to-end stint lifecycle?
- What is the smallest implementation sequence that proves the boundary before expanding configuration or UI?

## Deliverable

Produce a short integration review under this issue that:

- links the final telemetry design and revised profile design;
- identifies contradictions, duplicated state, and unsafe fallback paths;
- shows the complete launch/status/settlement flow for autonomous pi and interactive Claude;
- records the explicit human decision;
- after approval only, proposes independently reviewable implementation issues and their dependency order.

## Acceptance criteria

- Both prerequisite designs are complete and current before this review starts.
- No production implementation issue is spawned from either design before this checkpoint.
- Autonomous versus interactive eligibility is explicit, observable, and fails closed.
- Missing telemetry never becomes inferred success/failure or teardown authority.
- The combined design is reviewed with Jari and the decision is recorded.
- Implementation slices are filed only after approval, with shared hot surfaces sequenced rather than parallelized.

## Related work

- `@worker-telemetry-protocol`
- `@add-configurable-agent`
- `@end-end-stint`
