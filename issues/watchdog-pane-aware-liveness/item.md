---
created: 2026-07-27
updated: 2026-08-13
type: improvement
status: obsolete
priority: normal
closed: 2026-08-13
closed_by: adr-decision-2
---

# Watchdog liveness should key off pane_id in split interactive windows

## Description


Follow-up spun off from `capture-agent-pane-by-pane-id` (llm-review consensus: gemini + gpt-5.6).

## Problem
The supervisor's watchdog liveness probe keys off `TmuxIdentity.window_id`. In an interactive `/worktree-code` window the user has split, the agent's own pane (`%NN`) can exit/crash while a sibling shell pane keeps the *window* (`@NN`) alive. A window-existence probe then reports the node alive indefinitely — a zombie the supervisor never recovers. Correct for the single-pane autonomous/headless path (window ≈ pane there), so scoped out of the capture fix.

## Fix direction
When `TmuxIdentity.pane_id` is present, probe pane existence (and that the pane still belongs to the recorded window/session), not just window existence; fall back to `window_id` when `pane_id` is None (legacy runs). `kill-window` teardown stays window-scoped per state-integrity invariant #5 (supervisor owns the whole window). Tests: pane-gone-while-window-lives, pane-dead-with-`remain-on-exit`, pane-moved-to-another-window, legacy identity without pane_id.

## Where
`crates/taskfleet-cli/src/supervise/watchdog.rs`; `TmuxIdentity` already carries `pane_id` (`crates/taskfleet-core/src/schema.rs`).

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

The tmux pane-aware/tri-state liveness matrix is deleted as a primary signal — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
