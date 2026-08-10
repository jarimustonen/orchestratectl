---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: in-progress
priority: normal
labels: [from-homebase]
commits:
- hash: dc8b151047504793854c51c63f5aaf960be5f92a
  summary: surface supervisor at run show data top level
---

# run show --output json returns all-null data fields for a run that list/event-tail resolve

## Description

## Observed (low confidence — seen once)
For run `01kzn5b0mm377k6c7y3r9h0ebb` (a live spinoff), `orchestratectl run show <id> --output
json | jq '.data'` returned every field null:
`{kind:null, status:null, title:null, created_at:null, node_count:null, supervisor:null}`
— and `.data.supervisor.pid` was null — even though **the same run resolved fine** via
`orchestratectl run list` (showed it ALIVE with title/kind) and `orchestratectl event tail
<id>` (full event log: run.created, node.created, supervisor.started pid 65745). The actual
supervisor + agent PIDs were alive per `ps`.

## Expected
`run show --output json` should return the same populated data `run list` / `event tail` see,
or a clear error — not a silent all-null payload for a resolvable, live run.

## Notes
Observed once, during another repo's run being active; possibly a race (record mid-write) or a
resolution path that differs from `list`/`event tail`. Filing for awareness; reproduce before
fixing. Run state is global (`~/.orchestratectl/runs/`), so cwd/repo shouldn't matter.
