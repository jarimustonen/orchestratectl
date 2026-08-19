---
created: 2026-08-19
updated: 2026-08-19
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
---

# run create omits source_repo from fresh run manifest

## Description

run create omits source_repo from fresh run manifest

Observed

From `/Users/jari/Sources/homebase`, this command created a valid autonomous run and worktree:

`orchestratectl run create --kind spinoff --headless --source-branch main --title homebase-i2-characterization --prompt-file /tmp/homebase-i2-characterization.md --output json`

The returned worktree was `/Users/jari/Sources/homebase__worktrees/wt-01m0cb7v02-homebase-i2-characterization`, but `orchestratectl run show 01m0cb7v02kdftt8vy3gz4390e --output json` reported `.data.manifest.source_repo: null`. The field remained null after the run landed successfully.

Impact

`orchestratectl run list` is global across repositories. Stint orchestration must enumerate live/resumable runs and map each relevant run to the current repository before reconstructing issuectl lane/collision reservations. A missing source repo forces inference from an optional worktree path and becomes ambiguous after teardown or for malformed/stillborn runs.

Expected

`run create` should persist the canonical source repository path in `manifest.source_repo` whenever it successfully resolves the source branch and creates a worktree. `run show` and `run list` should expose that durable value after teardown.

Version

`orchestratectl 0.4.1` (`8777b2e3c5b891abf396c6486c9e81e17ffcfe85`).
