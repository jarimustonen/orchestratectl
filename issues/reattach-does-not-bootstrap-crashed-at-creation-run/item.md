---
created: 2026-07-18
updated: 2026-07-18
type: bug
reporter: jari
status: open
priority: normal
related: ['@cancel-dead-supervisor-recovery']
---

# run reattach does not bootstrap a child that crashed at creation (0 nodes)

_Source: crates/octl-cli/src/run/ (reattach path)_

## Description

Observed while driving an 8-unit headless fan-out from homebase (orchestratectl 0.1.0, commit a54f0ff, macOS).

## Symptom
A fan-out child run crashed **before spawning its worker node**: its event log contained only a single `run.created` event (seq 1), `run show` reported `nodes: 0`, `worktree_root: null`, and the supervisor was dead. `run reattach` on it reported success and started a fresh supervisor (new pid, `alive: true`), but the worker node was **never bootstrapped** — `nodes` stayed 0 across ~50s of polling (4x12s). Reattach kept a supervisor alive but did not (re)spawn the initial worker node for a run that never got past creation, so it is not an effective recovery path for the crashed-at-creation case.

Recovery that did work: `run cancel` + manual `git worktree remove --force` + `git branch -D` of the (empty, still at base commit) child branch, then re-`run create` with a fresh `--idempotency-key` (the original key would idempotent-replay the dead run). The re-spawned run started its worker node normally and completed.

## Evidence (run ids from the session)
- Crashed child: `01kxrcg18jyk02hxtm9s5y87w0` — only `run.created` in `event tail`; `nodes: 0`; child branch `wt/01kxrcg18j-...-frondeo-monorepo` still at the base commit (never committed).
- After `run reattach 01kxrcg18j...`: supervisor pid 95169 alive, but `nodes` remained 0.
- Re-spawn (new key `...-retry2`): `01kxrq64pz4n3r2qh4a4n17fgp` — worker node spawned (`nodes: 1`) and completed/merged.

## Secondary observation (likely the already-known dead-supervisor family)
A later session teardown killed the driver's + 3 other children's supervisors, leaving those runs `status: pending` even though each child's deliverable was **already committed to the integration branch**. Recovery was manual: merge the integration branch to the source branch, then `run cancel` the orphaned pending runs and prune worktrees/branches. Overlaps with @cancel-dead-supervisor-recovery / the fixed supervisor-dead-teardown issues; noting it as a recurrence signal, not a separate ask.

## Caveat
The reattach-doesn't-bootstrap symptom was observed **once**, over a ~50s window; it is possible (though it did not appear so) that the node would have spawned given longer. Worth reproducing deterministically before fixing. Suggested angle: reattach's recovery helper (SupervisorView::probe + reattach::spawn_supervisor + ensure_report_consumer, per @cancel-dead-supervisor-recovery) may assume >=1 node already exists and only re-attach the report consumer, with no path to spawn node n-0001 when the run died before its first node.spawned.

Non-blocking for v0.1.0; a headless fan-out just needs the cancel+respawn workaround.

Claude-Session: https://claude.ai/code/session_01HWgHqnKFzZxoP82XzN6a5q
