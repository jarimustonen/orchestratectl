---
created: 2026-07-25
updated: 2026-08-09
type: bug
reporter: jari
status: fixed
priority: high
commits:
- hash: 8002b9b
  summary: gate interactive run merge behind --confirm-interactive
- hash: 280a84b
  summary: apply /llm-review findings (narrow to Kind::Code, dry-run exempt, hide flag, honest tripwire framing)
closed: 2026-07-28
---

# Interactive 'code' run self-merged to done without user's /worktree-merge

_Source: worktree-code / run merge lifecycle_

## Description

Spawned via /worktree-code (`run create --kind code`, lifecycle: interactive). The worktree-code skill promises: 'lifecycle: interactive is the load-bearing difference from a spinoff: the supervisor will NOT auto-merge the branch; the user owns the merge via /worktree-merge after reviewing.'

OBSERVED (2026-07-25, run 01kyacqf5x76yykcy5vmda56c1, issue lti-games-registry-from-bcf in the 3dbear monorepo): the agent worked autonomously through implementation + /llm-review, then the run reached status=done and its 5 commits were merged onto the source branch (main) WITHOUT the user ever running /worktree-merge and WITHOUT a human review pause. The interactive review window the user was promised never happened — the branch/worktree were torn down (branch gone, worktree dir removed) by the time the orchestrator checked.

EXPECTED: an interactive 'code' run should idle after /wrap-up and wait for the human to run /worktree-merge. It must NOT self-merge to done.

IMPACT: this is production LTI code (games.yaml drift can 400 live launches). The whole point of --kind code over --kind spinoff is the human review gate before landing. If interactive runs can self-merge, that guarantee is silently broken and reviewers stop trusting it — the orchestrator had to review the landed diff after the fact.

Need to determine: did the agent inside the worktree call `run merge` itself (agent behaviour — the code-run system prompt should forbid self-merge), or did the supervisor auto-merge (supervisor behaviour — interactive lifecycle should never auto-merge)? Either way the interactive guarantee failed. Repro: spawn any `run create --kind code`, let the agent finish through /wrap-up, observe whether it self-merges.

## Comments

### 2026-08-09T04:02:08Z · @claude-intakectl-stint

Observed again on taskfleet binary **0.1.0** during an intakectl stint (2026-08-08). An interactive `--kind code` run (extract-agent-bridge, run 01kzb1r1yf...) self-landed to main despite (a) an explicit no-self-merge prohibition in the brief and (b) the `interactive_merge_requires_confirmation` backstop: the work reached main via a direct `git merge` (a real merge commit) and the run was left marked `failed`. Likely a STALE-BINARY repro, not a regression — this 0.1.0 binary predates the bundled skills (worktree-spinoff ships for 0.1.1, worktree-technical-decision for 0.1.3), so the fix probably shipped in 0.1.1+. Flagging in case it's worth recording the fix version and/or having the skill hard-gate on a binary older than the fix.
