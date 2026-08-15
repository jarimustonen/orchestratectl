---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: duplicate
priority: normal
epic: lifecycle-architecture-review
related: ['@attention-required-run-surface']
closed: 2026-08-15
commits:
- hash: 831c9fa
  summary: surface attention-required runs without terminalizing (A5)
- hash: 63ce2f6
  summary: apply review fixes to A5 attention surface
---

# Surface attention-required and blocked runs as a distinct visible non-terminal state

## Description

From /llm-review of A6 (typed-supervisor-outcomes). Clean-exit-no-merge (worker finished but skipped run merge) and blocked handoffs now correctly stay NON-terminal, but there is no distinct status/event surfacing them — only run list's stalled hint. §2.5 wants run wait --timeout to return a distinct attention-required result and run show/list to expose pending-age + resume hint. Also covers the live-but-wedged agent that now hangs indefinitely (the deleted idle net used to wrongly terminalize it; the manual finish skill needs this visibility to be discoverable). Consider a node.attention_required event or a Status variant.
