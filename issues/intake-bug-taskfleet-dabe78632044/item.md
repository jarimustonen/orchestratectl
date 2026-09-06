---
created: 2026-08-15
updated: 2026-08-20
type: bug
reporter: jari
status: duplicate
priority: normal
closed_by: claude
closed: 2026-08-16
provenance: agent-homebase-wrapup
---

# run create timeout can leave a supervisorless pending run with no nodes

## Description

run create timeout can leave a supervisorless pending run with no nodes

During homebase stint 2026-08-15, two `taskfleet run create --kind spinoff --headless ... --output json` calls were invoked with a 30s caller timeout. Both commands timed out from the caller side, but each had already created a run directory and manifest.

Observed:
- Run ids: `01m01vz786ym7jtpvt3c8vj5cw` and `01m01vz78a00fr4vhwxdeybqp2`.
- `run show` reported `status: pending`, `landed: false`, `landed_method: unverified`.
- Supervisor was `pid: null`, state `not-recorded`, alive `false`.
- Manifest had `source_repo: null`, `source_branch: null`, `worktree_root: null`, `node_count: 0`.
- `events.jsonl` contained only `run.created`.
- No worktree or tmux worker existed. I had to `taskfleet run cancel` both runs and retry with a longer timeout.

Expected:
- Either `run create` should not leave a durable autonomous run until the minimum runnable state is materialized, or the partial state should be terminal/failed with a clear error and cleanup path.
- A caller timeout should not leave a normal-looking pending run that cannot be supervised and has no nodes.

Why it matters:
- A stint orchestrator can interpret pending as unsettled work, but there is nothing to wait on or merge.
- The recovery path is currently manual cancellation plus retry, and the run state does not explain the partial-create failure.

## Resolution

### 2026-08-16T15:33:38Z · @claude

Duplikaatti: @run-create-long-title-stillborn. Sama vika — ajo syntyy, mutta valvoja tai työntekijä ei. Viisi issueta kuvasi tätä eri kulmista; konsolidoitu pitkän otsikon tapaukseen, koska se on AINOA jolla on deterministinen toisto (pitkä --title → katkaistu haaranimi → tmux-window-not-found → stillborn). Muut kolme ovat sama oire ilman toistoa ja neljäs on saman vian jälkihoitoa. Havainnot säilyvät tässä issuessa; korjaa toistettava ensin.
