---
created: 2026-08-14
updated: 2026-08-20
type: feature
reporter: jari
status: duplicate
priority: normal
closed: 2026-08-15
provenance: agent-homebase-wrapup
---

# run show --output json surfaces terminal report as "none"; report lives…

## Description

run show --output json surfaces terminal report as "none"; report lives in nodes/n-0001.json .last_report

## Observed

After a spinoff settles, `taskfleet run show <run-id> --output json` returns `.data.report` (and `.data.nodes[0].report`) as the string `"none"`, even though the worker DID submit a terminal report via `run merge`. The actual report (summary, discussion_items, spinoff_proposals, wrap_up_recommendations) is stored on disk at `~/.taskfleet/runs/<run-id>/nodes/n-0001.json` under the `.last_report` key. `taskfleet node show "<run-id>:n-0001" --output json` also returned empty for `.data.report.*` in this session.

To read the terminal report I had to `jq '.last_report'` the node JSON file directly:
```
jq '.last_report | {summary, discussion_items, spinoff_proposals, wrap_up_recommendations}' \
  ~/.taskfleet/runs/<run-id>/nodes/n-0001.json
```

## Expected

`run show`/`node show --output json` should surface the settled node's `.last_report` in `.data.report` (or `.data.nodes[].report`) so a caller/orchestrator can read discussion_items / spinoff_proposals / wrap_up_recommendations from the documented CLI, without reaching into the run-dir file layout.

## Context

taskfleet 0.1.8, macOS. Hit repeatedly this session (4 spinoffs) while gathering terminal-report follow-ups for a stint handoff — the `landed` boolean IS surfaced correctly by `run show`/`run wait` (that part works great); only the structured report is not. Minor friction, not a blocker (the data is on disk), hence `feature` not `bug`.
