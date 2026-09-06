---
created: 2026-08-12
updated: 2026-08-13
type: improvement
status: obsolete
priority: normal
related: ['@agent-skips-run-merge-idle-pending']
closed: 2026-08-13
closed_by: adr-decision-2
---

# idle-unmerged net: sum process-tree CPU, not just the agent PID

_Source: crates/taskfleet-cli/src/supervise/watchdog.rs::pid_cpu_time_centis_

## Description

The idle-unmerged safety net's CPU activity clock (`watchdog::pid_cpu_time_centis`, consumed by `supervise::mod::cpu_activity_clock`) reads `ps -o time -p <pid>` — ONLY the agent PID's own cumulative CPU, not its descendants. So a CPU-bound CHILD (a long `cargo test`/`cargo build`, a compiler, a local model) that emits nothing to the tmux pane leaves all three activity clocks quiet: no commit, no pane byte, agent-PID CPU near zero. If that silent child runs past IDLE_UNMERGED_THRESHOLD (30 min), the net can synthesize a false `agent-idle-unmerged` terminal report on a genuinely-working run. Today the pane clock (agent.log mtime, bumped by streamed child output) is the backstop, but child stdout is often BLOCK-buffered when it detects a non-TTY (tmux pipe-pane is a pipe), so streaming isn't guaranteed. Fix: sum CPU across the agent's process group / descendant tree (walk `pgrep -P` on macOS, or /proc on Linux) so a busy child keeps the clock fresh. Surfaced by the 4-model /llm-review of the reopen fix (agent-skips-run-merge-idle-pending). Reduces reliance on the asserted 5%-core CPU floor and would make a tighter idle threshold safe.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

The CPU activity clock is deleted by the thin model — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
