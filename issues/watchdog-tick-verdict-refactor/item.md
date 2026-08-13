---
created: 2026-07-28
updated: 2026-08-13
type: improvement
reporter: jari
status: obsolete
priority: normal
related: ['@agent-skips-run-merge-idle-pending']
closed: 2026-08-13
closed_by: adr-decision-2
---

# Extract watchdog_tick per-failure-mode blocks into a WatchdogVerdict classifier

_Source: orchestratectl supervise (idle-unmerged review follow-up)_

## Description

Spun off from `/llm-review` of the idle-unmerged safety net (Opus finding #15/#8).

`watchdog_tick` (crates/octl-cli/src/supervise/mod.rs) now carries four tightly-coupled, sequentially-gated failure-mode blocks — liveness → reconcile-merged → death-with-retry → idle-unmerged — each with its own outside-then-under-lock probe + TOCTOU close + logging conventions. Each new mode adds another block; the structure is approaching unmaintainable.

Proposal: extract a `WatchdogVerdict` enum and a `classify(node, git, tmux_snapshot, now, ...) -> WatchdogVerdict` function so the tick loop becomes a single `match` (Alive | ReconcileMerged | DeadEmptyHanded | DeadRecoverable | IdleUnmerged | ...), each arm doing the lock+synthesize. Keeps every existing TOCTOU/state-integrity invariant; purely a structural refactor with no behavior change (assert via the existing watchdog test suite).

Sub-note (Opus #8): while here, thread the actual `last_report.reason` through `cleanup::record_branch_preserved` instead of the hardcoded "blocked report" string, so a conductor scanning `cleanup.branch_preserved` audit events can tell "genuinely blocked handoff" from "agent-idle-unmerged" without reading each node's report. (The `run show` distinction already works via `last_report.reason`; this is audit-log parity.)

Tech debt, not correctness — no user-visible change.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

Refactors the watchdog_tick inference core, which is deleted — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
