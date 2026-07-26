---
created: 2026-07-20
updated: 2026-07-26
type: bug
reporter: jari
status: in-progress
priority: normal
related: ['@supervisor-dead-merge-no-teardown', '@false-failed-after-merge', '@supervisor-watchdog-misfire-on-fresh-spawn']
---

# run merge leaves tmux window + worktree after mid-session agent-died on a long-lived interactive run (0.1.0 regression)

_Source: supervise (watchdog liveness) + run merge teardown_

## Description

## Description

**Reporter:** homebase worktree `wt/01kxv28qw5-file-bug-image-barrier` session (interactive `/worktree-code`).
**Reported:** 2026-07-20
**Version:** orchestratectl `0.1.0`
**Severity:** Medium — no data loss (the merge landed correctly on `main` and the worktree was *not* destroyed), but `run merge` returns a success/`terminal` envelope while the tmux window, worktree, and branch are silently left behind, and the `/worktree-merge` skill then tells the user "the window is being cleaned up automatically" — which is false.

This reproduces on `0.1.0` a combination already marked **fixed** in `@supervisor-dead-merge-no-teardown`, `@false-failed-after-merge`, and `@supervisor-watchdog-misfire-on-fresh-spawn` — so it is either a regression or those fixes do not cover the **long-lived interactive** trigger. Filing fresh with a full authoritative event-log trace rather than reopening, because the end-to-end path (mid-session `agent-died` on a ~1.5-day interactive run → supervisor exit without teardown → later explicit `run merge`) spans all three and is a distinct reproduction.

## Symptom

`orchestratectl run merge 01kxv28qw5v2p67synn9w2wb4c --report-file …` returned:

```json
{"schema_version":1,"data":{"run_id":"01kxv28qw5v2p67synn9w2wb4c","node_id":"n-0001",
 "branch":"wt/01kxv28qw5-file-bug-image-barrier","source":"main","merged":true,
 "report_seq":7,"supervisor":{"state":"terminal"}},"warnings":[]}
```

`merged:true` + `supervisor.state:"terminal"` reads as "merged and torn down". But afterwards:

- The agent is **still running inside the worktree** (this report is being filed from it).
- `git worktree list` still shows `…/wt-01kxv28qw5-file-bug-image-barrier` and the branch `wt/01kxv28qw5-file-bug-image-barrier` still exists.
- The tmux window (`@18`, `🏠 💻 wt-01kxv28qw5-file-bug-image-barrier`) is still open.
- `run show` reports `manifest.status: failed`, `supervisor: {pid: null, alive: false}`.
- `node show n-0001` `last_report` is `{failed: true, reason: "agent-died", via: null, success: false}` — NOT the `via: "explicit-merge"` report the merge appended.

## Evidence — `events.jsonl` for run `01kxv28qw5v2p67synn9w2wb4c` (UTC)

| seq | ts (UTC)            | kind                | data |
|-----|---------------------|---------------------|------|
| 1   | 2026-07-18 16:52:03 | `run.created`       | kind=code, lifecycle=interactive, source_branch=main |
| 2   | 2026-07-18 16:52:17 | `node.created`      | agent_pid=73901 |
| 3   | 2026-07-18 16:52:17 | `supervisor.started`| pid=74085 |
| 4   | 2026-07-20 05:28:29 | `node.report`       | **reason=agent-died** |
| 5   | 2026-07-20 05:28:29 | `run.status`        | → failed |
| 6   | 2026-07-20 05:28:30 | `supervisor.exited` | **reason=work-complete, pid=74085** |
| 7   | 2026-07-20 05:46:42 | `node.report`       | **via=explicit-merge** |

The agent was demonstrably **alive** across seq 4: it continued the session and issued `run merge` at seq 7 (05:46:42), ~18 minutes *after* the watchdog declared it `agent-died` at seq 4 (05:28:29).

## Three defects on the one trace

1. **Watchdog `agent-died` false positive on a long-lived interactive run (seq 4).** After ~1.5 days the liveness poll classified the still-alive agent (pid 73901) as dead and synthesized a terminal `failed`/`agent-died` `node.report`, terminalizing the node. `@supervisor-watchdog-misfire-on-fresh-spawn` covers the *fresh-spawn* window; this is the *long-running / long-idle* window and still misfires on `0.1.0`.

2. **Supervisor exits without teardown, mislabeled `work-complete` (seq 6).** One second after the synthetic `agent-died`, the supervisor exits with `reason=work-complete` (misleading — the node was `failed`, not complete) and does **not** close the tmux window / remove the worktree / delete the branch. For an interactive-kind run, suppressing destructive auto-cleanup on `agent-died` is arguably correct (it saved my uncommitted work), but it leaves no live consumer for any later terminal transition — and nothing records that teardown was skipped.

3. **`run merge` reports `merged:true` + `supervisor.state:"terminal"` into a dead supervisor (seq 7).** The merge appended its `via:"explicit-merge"` report (seq 7) but the node was already terminal (seq 4) and the supervisor already gone (seq 6), so nothing consumed it and no teardown ran. The success envelope has no signal that the supervisor was dead / that teardown did not happen. This is exactly `@supervisor-dead-merge-no-teardown` (marked fixed) — still reproducible on `0.1.0`. Because the node's terminal report stayed `agent-died`, `run show` also reports `status: failed` despite the branch being merged to `main` (the `@false-failed-after-merge` symptom).

## Impact

- The `/worktree-merge` skill trusts the success envelope and tells the user "the worktree + window are being cleaned up automatically." They are not — orphaned tmux windows, worktrees, and branches accumulate silently and must be reaped by hand.
- `run show` reports `failed` for a run whose work merged cleanly, inverting the truth for any human/agent that trusts it.

## Suggested fix directions

- `run merge` should detect a dead/absent supervisor (`supervisor.alive == false`) and either (a) perform teardown inline, or (b) surface it loudly in the envelope (`teardown: "skipped-supervisor-dead"` + a `warning`) instead of returning a bare `supervisor.state:"terminal"`. A later explicit `via:"explicit-merge"` terminal report that lands on an already-`agent-died` node should reconcile the node/run status (branch-merged ⇒ not `failed`) rather than being silently dropped.
- The watchdog liveness check should not classify a long-lived interactive agent as `agent-died` on a transient/idle poll (re-verify before terminalizing; treat interactive-kind more conservatively).
- `supervisor.exited reason=work-complete` should not be emitted when the terminating transition was an `agent-died` failure — the reason is misleading in the log.

## Reproduction

1. Start an interactive `/worktree-code` run; keep it alive across a long/idle span (here ~1.5 days) until the watchdog misfires `agent-died` (seq 4) and the supervisor exits (seq 6).
2. In the same still-alive agent, commit and run `orchestratectl run merge <run-id> --report-file …`.
3. Observe: `merged:true` + `supervisor.state:"terminal"` returned, but the tmux window, worktree, and branch survive and `run show` reads `status: failed`.
