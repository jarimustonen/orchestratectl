---
created: 2026-07-24
updated: 2026-08-16
type: bug
status: obsolete
priority: normal
closed: 2026-08-16
closed_by: claude
---

# Worker Claude process can hang mid-run; run reports failed though work is committed-unmerged

_Source: run wait / run merge_

## Description

## Observed (real stint, 2026-07-24, glasspad html-artifact-host-rewrite Wave 3a)

A spinoff worker's Claude process hung mid-run. The agent had COMMITTED its work to the worktree branch (one commit; build+clippy+tests+security-suite all green) but DIED before running 'taskfleet run merge'.

'taskfleet run wait <id>' returned: status=failed, merged=false, summary='Agent for node n-0001 stopped responding: agent-died', error='agent-died'. Supervisor left the worktree+branch in place (correct for a terminal failed run).

## Impact

The failed/agent-died status does not distinguish (a) worker died with no usable work from (b) worker committed complete, green work but hung before the merge step. The orchestrator had to salvage manually: verify branch green -> 'git merge --ff-only <branch>' -> 'git worktree remove --force' + 'git branch -d' -> spawn a separate deferred /llm-review spinoff to close the skipped review gate. Undocumented manual recovery for a common-enough failure (one hang in a 7-worker session).

## Ideas

- Heartbeat/watchdog on the worker process so a hang is reported as 'hung' (distinct from clean agent-died), with a configurable timeout + optional auto-resume.
- 'run wait'/'run show' should surface whether the worktree branch has commits ahead of source on a failed run, so callers can tell salvageable from empty.
- A first-class 'taskfleet run salvage <id>' (or 'run merge --force-from-branch') that merges a dead run's committed branch and tears down, instead of hand-rolled git.

## Related

BUG-false-failed-despite-successful-merge (run status unreliable: settled != landed) — same theme of run status not reflecting git reality.

## Corroboration (2026-07-26 stint, wave 2) — deterministic ~13-min death on a heavy-LLM task; NO diagnostic trace

An autonomous `--kind spinoff` running the `pipeline-tiered-triage` task died via
`agent-died` **twice in a row**, deterministically, ~13 min after supervisor start,
having committed **nothing** (branch tip == base_sha both times; empty worktree
preserved by the blocked-report gate). Runs `01kyea060t` (r1) and `01kyec2pne` (r2);
r2 supervisor.stderr.log: `{"reason":"work-complete","iterations":676}` (676 polls
≈ 12m56s: `supervisor.started` 04:49:11 → `node.report reason=agent-died` 05:02:07).

Distinguishing facts vs the original report:
- This is a **pre-merge** death with **no committed work** (not the salvageable
  commit-then-hang case) — so it's not recoverable by `run merge --force-from-branch`;
  the work simply never happened.
- Confirmed **NOT** the watchdog false-positive: `agent-died-merge-no-teardown-interactive`
  fixed the *interactive* liveness path (tmux-window authoritative) but left autonomous
  **pid-authoritative** and correct — so for an autonomous run, `agent-died` means the
  claude PID genuinely exited. The agent process really died.
- Reproducible only on the **heavy-LLM** task (long spec/verify + `/llm-review`); the
  mechanical Rust fixes in the same wave (`child-supervisor`, the watchdog fix itself)
  survived fine. Suggests a duration/context/API-driven death specific to long agent runs.

### Biggest blocker to diagnosis: autonomous agent output is not captured anywhere durable
The agent's stdout/stderr goes only to its tmux pane, which the supervisor kills on
cleanup — so a genuine death leaves **zero trace** of the cause (context exhaustion? an
API error/cutoff? a crash?). The run dir has only `events.jsonl` / `manifest.json` /
`supervisor.stderr.log`, none of which carry the agent's own output. **Fix direction:**
capture the agent pane to a durable `<run-dir>/agent.log` (e.g. `tmux pipe-pane` set up
by the supervisor right after spawn confirmation) so post-mortem diagnosis is possible.
Until that exists, every autonomous worker death is uninvestigatable after teardown.

## Resolution

### 2026-08-16T15:32:30Z · @claude

Ohitettu `run salvage` -komennolla (A3, design §2.2). Jumittunut työntekijä näkyy nyt attention-required -tilassa (run list/show/wait) ja pelastuskomento vie säilytetyn työpuun maaliin; issuen kuvaama käsin tehty elvytys (git merge --ff-only + worktree remove + branch -d) ei ole enää tarpeen. Verifioitu koodista (run/salvage.rs).
