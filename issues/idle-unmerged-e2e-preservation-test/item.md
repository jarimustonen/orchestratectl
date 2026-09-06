---
created: 2026-08-12
updated: 2026-08-13
type: task
status: obsolete
priority: normal
related: ['@agent-skips-run-merge-idle-pending']
closed: 2026-08-13
closed_by: adr-decision-2
---

# e2e test: idle-unmerged synthesized report preserves branch+worktree through cleanup

_Source: crates/taskfleet-cli/tests/e2e_spinoff.rs_

## Description

The idle-unmerged safety net (issue agent-skips-run-merge-idle-pending) is covered by unit tests over cpu_activity_clock + node_idle_unmerged + a single watchdog_tick, but NOT end-to-end through the real supervisor cleanup path. Add an e2e test (extend tests/e2e_spinoff.rs) that: drives a live autonomous run whose stub agent commits then idles without run merge, lets the watchdog synthesize the agent-idle-unmerged report, then runs the supervisor cleanup tick, and asserts invariant 5 holds — branch still exists, worktree still exists, committed work reachable, NO force-delete (no via marker), tmux window closed. Also assert a concurrent worktree-dirtying between the clean-worktree snapshot and cleanup causes preservation, and that a real terminal node.report landing in the TOCTOU window is NOT overwritten by the synthetic failure. Surfaced by /llm-review (OpenAI #9/#10) — the preservation gating is currently asserted only by comments + indirect unit checks.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

Tests the synthesized idle-unmerged report path (deleted); branch-preservation now covered by the A6 typed-outcome table — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
