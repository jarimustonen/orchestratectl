---
created: 2026-07-25
updated: 2026-07-28
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
