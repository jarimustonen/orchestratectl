---
created: 2026-06-28
updated: 2026-06-29
type: bug
status: fixed
priority: high
closed: 2026-06-29
---

# Orchestrated child worktree cut from main, not from --source-branch (breaks DAG dependencies)

_Source: create.sh worktree base_

## Description

During the /orchestrate smoke test (2026-06-28), orchestrated children were spawned with `--source-branch orchestrate/smoke-demo-2026-06-28` (the integration branch), but their worktrees were cut from `main`, NOT from the named source branch.

Evidence:
- Feature A worktree commit (c17d987) was parented on main`s tip (117fff6), not on the integration branch tip (e470ea2) it was supposedly forked from.
- Feature C`s own node-report wrap-up: "my worktree branch was cut from main before A/B merged, so I had to merge orchestrate/smoke-demo-2026-06-28 into my worktree to obtain the scripts before writing the README."

Impact: for a dependency-ordered DAG, downstream children do NOT see upstream features` merged outputs at worktree-creation time. The campaign only succeeded because each agent diligently merged the integration branch into its own worktree first. If an agent forgets, it writes against stale/missing dependencies. This defeats the core value of /orchestrate (dependency-ordered features off a shared integration branch).

Expected: `--source-branch <branch>` should be the fork point of the child worktree. Found during /orchestrate end-to-end smoke test.
