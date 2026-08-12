---
created: 2026-08-12
updated: 2026-08-12
type: epic
owner: jari
status: open
priority: normal
---

# Re-examine the run/supervisor/agent lifecycle architecture

## Description

~57% of open issues (and 58% of bugs) cluster in the run/supervisor/agent-lifecycle subsystem. Hypothesis: the supervisor INFERS a distributed process's lifecycle from indirect signals (pid liveness, tmux pane, git branch, node report) — a polling-inference model whose edge cases are combinatorial, so patching never shrinks the list. This epic reviews the current design feature-by-feature, audits unused-complexity drag, surveys protocol/state-machine alternatives, and drives a keep-and-harden vs re-architect decision to an ADR. Evidence: DAG cluster analysis 2026-08-12.
