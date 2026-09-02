---
created: 2026-09-02
updated: 2026-09-02
type: feature
reporter: jari
status: untriaged
priority: normal
provenance: agent:3dbear-stint-handoff
source_ref: agent:3dbear-stint-handoff/reporter:jari/id:failed-run-preserved-worktree-teardown-20260902
---

# Add teardown for terminal failed runs with preserved worktrees

## Description

Add teardown for terminal failed runs with preserved worktrees

## Observed

Three superseded runs were terminal `failed`, had no live supervisor, and retained clean worktrees and branches. `orchestratectl run cancel` could not relinquish or remove them:

```text
orchestratectl run cancel 01m1em8me7kq1pbapjfrsxnfx1 --output json
{"error":{"code":"run_already_terminal","message":"run is failed, cannot cancel",...}}
```

`run salvage` was inappropriate because the branches were intentionally superseded and must not be merged. The only available cleanup was manual `git worktree remove` plus `git branch -D`, which leaves orchestratectl's manifest unable to record the explicit abandonment/cleanup decision.

## Expected

Provide a safe command such as `orchestratectl run abandon <run-id>` or `run cleanup <run-id>` for terminal failed runs. It should:

- require a terminal failed/cancelled state and no live worker;
- refuse dirty worktrees unless an explicit reviewed override is supplied;
- remove the preserved worktree and branch without merging;
- retain the run manifest, report, and event history;
- record who abandoned it, when, and why;
- be idempotent and return structured JSON;
- clearly distinguish cleanup from cancellation and salvage.

This is needed by terminal handoff workflows so superseded failures can be closed without direct git surgery.
