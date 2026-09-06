---
created: 2026-08-22
updated: 2026-08-24
type: task
reporter: jari
status: done
priority: normal
related: ['@worker-telemetry-protocol', '@add-configurable-agent', '@end-end-stint']
lane: lifecycle
lane_seq: 50
collision: [run-create]
blocked_by: ['@worker-telemetry-protocol', '@add-configurable-agent']
review_status: approved
commits:
- hash: 695e82d
  summary: Prepare worker control-plane approval checkpoint
- hash: 3b1e20e
  summary: 'docs: simplify worker control-plane design'
closed: 2026-08-24
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
- Which state belongs to the pi adapter, the harness-neutral taskfleet protocol, and the end-to-end stint lifecycle?
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

## Agent Runs

### 2026-08-23T07:35:48Z · @pi

Prepared integration-review.md from both prerequisite designs. Four-model /llm-review with two cross-review rounds was assessed; all eight surviving findings were corrected. The document remains a recommendation-only checkpoint, leaves all five telemetry candidates untriaged and unchanged, creates no profile slices, and requests Jari’s explicit approve / amend / reject decision.

## Decisions

### 2026-08-24T07:39:07Z · @jari

Approved with simplifications on 2026-08-23. Binding product decisions: (1) remove the agent-permission/operation-set model; agents have full normal rights; (2) telemetry is a keep-it-simple advisory feature that tells the calling agent last reported activity and freshness so that caller can judge the situation—telemetry does not itself become success truth; (3) initially only pi with the adapter is autonomous, while Claude remains explicit-interactive; (4) fallback never weakens residency or telemetry requirements; (5) the existing local secure profile is usable now without special enforced restrictions, and tighter enforcement may come later; (6) executable agent commands live only in user-owned config; (7) requested and selected agent choice is plainly visible; (8) agent failure disclosure is accepted. Revise the source designs and implementation split to this simpler scope before implementation.

## Resolution

### 2026-08-24T08:09:00Z · @issuectl

Applied the 2026-08-23 simplification to both source designs and the integration review. Four-model review plus two cross-review rounds found eight justified fidelity gaps; all were corrected without restoring rejected permission, trust, launch-enforcement, or provenance complexity. The five telemetry candidates remain untriaged and unchanged, and the profile slice remains proposal-only.
