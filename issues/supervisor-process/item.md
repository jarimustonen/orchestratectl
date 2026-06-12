---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: done
priority: high
epic: orchestratectl-mvp
commits:
- hash: 2b8793e
  summary: 'feat(supervisor-process): supervise subcommand + reattach upgrade'
- hash: 9a4e598
  summary: 'test(supervisor-process): V2/V3/V7/V8/V9 validation gates'
- hash: 6d2093a
  summary: 'docs(supervisor-process): handoff notes'
closed: 2026-06-12
---

# Supervisor process (orchestratectl supervise)

## Description

orchestratectl supervise <run-id> long-lived subcommand. Tail-follow loops on (a) own run events, (b) each child run's events (from children registry). On child.spawned event in own log: fork+exec a child supervisor for the new child run, record supervisor_pid in child node JSON, add to tracking set. On node.report in a child run's log: process per §7.3 with deterministic-ID dedup. Agent liveness via dual polling (kill(agent_pid, 0) + tmux window presence + start-time identity defense) per §7.5. **No global SIGCHLD handler** — would conflict with std::process::Command. SIGINT/SIGTERM trap (via ctrlc crate) with supervisor.exited event + clean PID file removal. Records supervisor.pid. **Depends on** state-schema-crate. **Validation gates**: V2 (tmux pane PID discovery), V3 (kill+start-time identity), V7 (deterministic-ID dedup), V8 (run reattach), V9 (run cancel propagation).
