---
created: 2026-08-14
updated: 2026-08-20
type: feature
reporter: jari
status: wontfix
priority: normal
labels:
- via:agent-homebase-wrapup
closed: 2026-08-14
closed_by: stint-orchestrator
---

# Auto-land an idle spinoff whose work is committed and merges cleanly

## Description

Auto-land an idle spinoff whose work is committed and merges cleanly

Observed 2x on 2026-08-13: an autonomous spinoff committed complete work that merges
cleanly but the agent went idle without calling run merge. The supervisor terminalizes the
run failed + recoverable (agent-idle-unmerged, "land it with run merge"), but landing then
requires a manual orchestratectl run merge <id>.

Idea: when the recoverable state is unambiguous (branch present, merges_cleanly true,
unmerged_commits > 0, agent idle > threshold), AUTO-land it (submit minimal terminal report
+ merge) or add run merge --recover-idle / a config opt-in, instead of manual recovery.
Reduces babysitting of fire-and-forget spinoffs. Low priority; a larger arch refactor is in
flight there - reconcile rather than land a competing change.

## Resolution

### 2026-08-14T12:01:32Z · @stint-orchestrator

Jari (2026-08-14, handoff intake gate): close as subsumed by the thin-supervisor ADR (docs/decisions/0001), which deliberately chose manual finish (attention-required) over auto-anything for exit-without-merge. Not meaningful for 0.2.0 orx as currently framed. Re-file if a concrete need surfaces later.
