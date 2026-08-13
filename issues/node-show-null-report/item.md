---
created: 2026-08-12
updated: 2026-08-13
type: bug
reporter: jari
status: open
priority: normal
labels: [keep-0.2]
---

# node show returns null report after spinoff self-merge (report is in nodes/<node>.json last_report)

## Description

## Summary

`orchestratectl node show <RUN_ID> <NODE_ID> --output json` returns `.data.report: null` after a spinoff has self-merged (`run merge`), even though the structured terminal report was submitted and IS persisted on disk. The report lives in `~/.orchestratectl/runs/<RUN_ID>/nodes/<NODE_ID>.json` under the key **`last_report`** (with `success`, `summary`, `discussion_items`, `spinoff_proposals`, `wrap_up_recommendations`, `via`).

## Observed

```
$ orchestratectl node show 01kzrp7ak2xsj2j2nqqf3jnhce n-0001 --output json | jq '.data.report'
null

$ jq '.last_report | keys' ~/.orchestratectl/runs/01kzrp7ak2xsj2j2nqqf3jnhce/nodes/n-0001.json
["discussion_items","spinoff_proposals","success","summary","via","wrap_up_recommendations"]
```

The `last_report` object contained the full report — a rich `summary`, two `spinoff_proposals`, and two `wrap_up_recommendations`. `node show` surfaced none of it (`.data.report` was `null`; a sibling `run show … .data.nodes[0]` had `id`/`status`/`report` all `null` too).

## Expected

`node show` should surface the node's terminal report. The `worktree-spinoff` skill documents it as the canonical way to read that report:

> `orchestratectl node show <node-id>` — the structured terminal report `orchestratectl run merge` submits as it merges the branch.

Today a caller has to fall back to reading the raw `nodes/<node>.json` `last_report` key, which is undocumented and couples the caller to the on-disk layout.

## Impact / priority

Low — the data isn't lost, and `run wait` folds the report `summary` into its JSON (that path works). But the two documented follow-up channels for reading discussion_items / spinoff_proposals / wrap_up_recommendations after teardown (`node show` / `run show … nodes[]`) return null, so an orchestrator that relies on them to harvest a spinoff's proposals will silently see nothing.

## Environment

orchestratectl 0.1.5 (commit 4baffdd1). Reproduced on 3 separate completed spinoff runs in one session.

## Decisions

### 2026-08-13T11:10:43Z · @adr-decision-2

KEEP-and-fix: The terminal-report surface survives; reading last_report correctly is a model-independent read bug. Surface survives the thin model; fix is model-independent. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
