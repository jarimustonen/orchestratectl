---
created: 2026-08-17
updated: 2026-08-17
type: feature
reporter: jari
status: duplicate
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
closed: 2026-08-17
---

# run create: per-run worker model override (harness args), without mutat…

## Description

run create: per-run worker model override (harness args), without mutating global pi settings

Observed (ossctl stint #22, 2026-08-17): orchestratectl run create only exposes --harness claude|pi. The pi harness reads its model from ~/.pi/agent/settings.json, so choosing a specific worker model per spawn (e.g. escalating a failed unit from gpt-5.6-terra to gpt-5.6-sol) required temporarily rewriting the user's GLOBAL pi settings before run create and restoring them after the agent started — racy (a concurrent spawn inherits the wrong model) and easy to forget to restore.

Expected: a per-run override on run create, e.g. --harness-arg / --model passed through to the launched agent command (pi supports --model "provider/id:<thinking>" on its CLI), recorded on the run manifest so run show displays which model a worker ran on.

Context: the escalation was needed because a terra worker gave up twice on a large semantic seam; the sol worker finished it in one pass. Per-unit model selection is a real orchestration lever, not a nice-to-have.

## Comments

### 2026-08-17T10:44:46Z · @orchestrator

Closed as duplicate 2026-08-17 of @add-configurable-agent, which covers per-run worker selection as part of named capability profiles. Nothing was discarded: all three of this report's distinct requirements (per-run override as the primitive and a valid MVP slice, recording the resolved model on the manifest + run show, and the racy global-settings workaround it must replace) were transcribed onto that issue verbatim before closing, along with the pi `--model 'provider/id:<thinking>'` syntax datum and the terra→sol escalation context. Closed rather than laned because both target the same run-create + harness::select surface, and two issues on one hot surface is the collision shape that has broken integrated main twice in this repo.
