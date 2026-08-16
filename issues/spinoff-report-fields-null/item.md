---
created: 2026-08-16
updated: 2026-08-16
type: bug
reporter: jari
status: open
priority: high
---

# Spinoff terminal report fields persist as null

## Description

## Description

Spinoff workers reliably reach `run merge` and land clean work, but the structured §7.3
report fields (`summary`, `discussion_items`, `spinoff_proposals`,
`wrap_up_recommendations`) arrive **null** in `nodes/n-0001.json` — even when the worker's
brief explicitly instructs it to write a populated `--report-file` payload.

Observed 4/4 times in a single `/stint-start` round on the `project-canon` repo
(2026-08-16). All four runs merged successfully; all four persisted a null report.

## Evidence

Four consecutive spinoff runs, each briefed (with escalating explicitness — run 3 and 4
carried a "please actually populate these fields" instruction naming the prior failures):

| run_id | title | status | landed | report fields |
|---|---|---|---|---|
| `01m05ejk7bc5g85tj2jjkxmkzh` | normalize-json-error | done | true | all null |
| `01m05fe6fgcw78s55vppdp37fa` | add-version-json | done | true | all null |
| `01m05g9pnqmj9xq5prd5rcmdkv` | help-json-canon-14 | done | true | all null |
| `01m05gyzvgns7m3m879yahtjv2` | config-surface-canon-8 | done | true | all null |

Probe used per run:

```bash
jq -c '{summary: .report.summary, discussion: .report.discussion_items,
        spinoffs: .report.spinoff_proposals, wrap: .report.wrap_up_recommendations}' \
  ~/.orchestratectl/runs/<run-id>/nodes/n-0001.json
# → {"summary":null,"discussion":null,"spinoffs":null,"wrap":null}
```

**Notably, `run wait` DID return a rich, accurate `summary` string for every run** — e.g.
for the last run: `"Delivered config path/show with TOML defaults→file→env provenance,
redaction-ready JSON, help/version integration, and review hardening; self-review found 0
confirmed gaps and doctor is mechanically conformant."`

So a summary exists somewhere in the pipeline and is surfaced by `run wait`, while
`nodes/n-0001.json`'s `report.summary` is null. That mismatch is the strongest clue: this
looks less like "the worker never wrote a report" and more like the report (or at least its
summary) is captured but **not persisted into the node record** — or is persisted to a
different path/shape than the one `node show` and the node JSON read.

## Why it matters

The structured report is the only channel by which an autonomous worker hands *reasoning*
back to its orchestrator: canon-interpretation calls it made, follow-up work it noticed,
warnings for the next unit in the lane. With it null, a `/stint` conductor must reconstruct
every round fact from `git log` and diff inspection — which recovers *what* changed but
permanently loses *why*, and loses `spinoff_proposals` entirely. In the round above, three
units were explicitly asked to flag anything the next unit in the same serial lane needed
to know; none of that reached the orchestrator.

## Suspected area

Worth checking in this order, given the `run wait` / node-record mismatch:

1. Whether `run merge --report-file <path>` actually parses and persists the file's rich
   fields, or only threads `success` + `summary` through to the supervisor.
2. Whether the auto-report path (`run merge` with no `--report-file`) **overwrites** a
   previously-submitted report with a minimal `{success, summary}` — and whether workers are
   silently falling back to it.
3. Whether `report.summary` in `nodes/<node>.json` is written from a different source than
   the summary `run wait` returns.
4. Whether a schema/field-name mismatch causes silent drops (the skill docs warn that unknown
   keys like `discuss` / `wrap_up` pass validation but are dropped — if the persisted shape
   drifted from the documented one, every field would null out exactly like this).

Also worth confirming: `orchestratectl node show n-0001 --run-id <id> --output json` returned
**no output at all** during triage, which may be a second, related surfacing bug.

## Acceptance

- A spinoff that submits a populated `--report-file` has all four fields readable afterwards
  via both `node show` and `nodes/<node>.json`.
- A worker that submits an empty/auto report is distinguishable from one whose report was
  dropped — silent nulls are never the outcome of a successful `run merge --report-file`.
- If a submitted report cannot be persisted, `run merge` says so loudly rather than merging
  and reporting success.
- Regression coverage for the populated-report round-trip.

## Comments

Filed from the `project-canon` repo after a 4-unit `/stint-start` round (2026-08-16) where
all four workers did correct work and merged cleanly — the defect is purely in the report
channel, not in worker behaviour or the merge path. Reported at the maintainer's request
after the pattern proved systemic rather than prompt-fixable.
