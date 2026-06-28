---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
commits:
- hash: 088d63f
  summary: 'refactor(core): typed node id + reducer plan-then-commit + corrupt-state error class'
---

# Reducer: collapse validate_event/apply_event into one plan-then-commit path

## Description

Multi-model review of event-log-durability-trio (all 4 reviewers) flagged the validate_event/apply_event mirror as the top maintainability risk: two parallel branch-for-branch implementations that must stay in lockstep or a poison line ships. Refactor each apply_* into a pure plan phase that reads state + validates + returns the projection writes to make (e.g. reduce_event_to_ops(paths, ev) -> Result<Vec<ProjectionOp>>); validate_event = plan-and-discard, apply_event = plan-then-write. Eliminates drift by construction and halves projection reads. Guarded today only by the validate_event_agrees_with_apply_event battery (now covers all 11 kinds). Source: history/review-event-log-durability-trio.md (consensus #1).
