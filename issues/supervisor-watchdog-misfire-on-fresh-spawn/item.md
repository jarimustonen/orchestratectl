---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: in-progress
priority: normal
commits:
- hash: daca558
  summary: grace window stops watchdog mis-firing on fresh spawns
---

# Watchdog mis-fires agent-died on fresh spawn, destroying agent's work

## Description

Symptom: a freshly-spawned autonomous run's watchdog can synthesize an `agent-died` terminal `node.report` within ~milliseconds of `node.created`, before the real agent has had a chance to start emitting work. The node is terminalized as `failed`, and the new supervisor-rollup (commit ed99cc7) then transitions the run to `failed` — even though the agent is alive, will go on to do hours of work, commit, merge, and submit its own real terminal `node.report` later. The agent's real report is dropped because the node is already terminal.

Concrete trace from run 01kw7e3brbe94vbr6p5gmqzf4a (the spinoff that implemented the rollup + auto-cleanup fix itself, 2026-06-28):

```
[1] 15:38:19.275 run.created title=supervisor-terminal-cleanup
[2] 15:38:20.803 node.created node=n-0001
[3] 15:38:20.837 supervisor.started                            # +34ms from node.created
[4] 15:38:20.862 node.report node=n-0001 reason=agent-died     # +25ms from supervisor start
[5] 16:02:02     node.report node=n-0001                       # 23+ minutes later: real terminal report from the agent
```

`node show n-0001` last_report shows the watchdog's synthetic report (success:false, reason:agent-died, summary "Agent for node n-0001 stopped responding"), not the real one — because the node was already terminal when the agent's real report landed, the reducer (presumably correctly) refused to overwrite it.

Root cause hypothesis:
- The watchdog reads `node.agent_pid` from the projection and pings it (`kill(pid, 0)` or similar).
- On a very fresh node, there's a window between `node.created` (which records the discovered PID from create.sh) and the OS having that PID actually mapped to the spawned `claude` process — or some race where the PID has been recycled or the agent forks immediately and the recorded PID is the launcher that exits.
- Watchdog sees "PID not alive" -> synthesizes terminal report.

Consequence with the new auto-cleanup landed:
- The supervisor will roll the run to `failed` based on the synthetic report.
- Auto-cleanup will then close the (autonomous-kind) tmux window, remove the worktree, delete the branch — DESTROYING the agent's still-running work mid-flight.
- This makes the bug now actively destructive, not just cosmetic. Pre-fix the watchdog mis-fire was annoying but harmless because the user manually saw the agent was still alive and let it finish.

Fix direction:

1. Watchdog must guard against fresh nodes — refuse to fire `agent-died` within some minimum age window (e.g. `now() - node.created_at > 5s`) so the OS has time to map the PID and the agent has time to checkpoint that it's alive.
2. OR: watchdog requires N consecutive failed pings before firing, not one (so a transient PID-discovery race doesn't terminalize).
3. OR: combine — short grace period AND multiple consecutive failures.
4. Verification: explicitly assert PID is alive AFTER node.created event landed (e.g. in `verify_agent_pid` re-check at supervisor start) and emit `node.failed` immediately if not — then the failure mode is "spawn never had a live agent" (correct semantics) rather than "spawn had a live agent we mis-pinged".

After this fix, also confirm:
- The recently-landed auto-cleanup does NOT fire on the synthetic `agent-died` if the watchdog has been suppressed during the grace window (because there will be no terminal node.report to act on).
- The auto-cleanup DOES still fire on legitimate `agent-died` (when the agent really crashes after a real grace period).

Related:
- `crates/octl-cli/src/supervise/mod.rs` around line 1162 — the `!terminal` guard on watchdog-synthesized reports
- `crates/octl-cli/src/run/spawn.rs::verify_agent_pid` — the pre-fork PID check; may need to be re-run at supervisor start
- Probably worth coordinating with the throwaway-repo harness issue (`spinoff-throwaway-harness`) because that harness is the cleanest way to test "many fast spawns + watchdog doesn't false-alarm"

Severity: HIGH. Without this fix, every spinoff that takes a normal amount of time (minutes) risks having its work destroyed by auto-cleanup mid-flight. Workaround for now: trust `git log main` (work IS committed) and ignore `run show` status.
