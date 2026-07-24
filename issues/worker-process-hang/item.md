---
created: 2026-07-24
updated: 2026-07-24
type: bug
status: open
priority: normal
---

# Worker Claude process can hang mid-run; run reports failed though work is committed-unmerged

_Source: run wait / run merge_

## Description

## Observed (real stint, 2026-07-24, glasspad html-artifact-host-rewrite Wave 3a)

A spinoff worker's Claude process hung mid-run. The agent had COMMITTED its work to the worktree branch (one commit; build+clippy+tests+security-suite all green) but DIED before running 'orchestratectl run merge'.

'orchestratectl run wait <id>' returned: status=failed, merged=false, summary='Agent for node n-0001 stopped responding: agent-died', error='agent-died'. Supervisor left the worktree+branch in place (correct for a terminal failed run).

## Impact

The failed/agent-died status does not distinguish (a) worker died with no usable work from (b) worker committed complete, green work but hung before the merge step. The orchestrator had to salvage manually: verify branch green -> 'git merge --ff-only <branch>' -> 'git worktree remove --force' + 'git branch -d' -> spawn a separate deferred /llm-review spinoff to close the skipped review gate. Undocumented manual recovery for a common-enough failure (one hang in a 7-worker session).

## Ideas

- Heartbeat/watchdog on the worker process so a hang is reported as 'hung' (distinct from clean agent-died), with a configurable timeout + optional auto-resume.
- 'run wait'/'run show' should surface whether the worktree branch has commits ahead of source on a failed run, so callers can tell salvageable from empty.
- A first-class 'orchestratectl run salvage <id>' (or 'run merge --force-from-branch') that merges a dead run's committed branch and tears down, instead of hand-rolled git.

## Related

BUG-false-failed-despite-successful-merge (run status unreliable: settled != landed) — same theme of run status not reflecting git reality.
