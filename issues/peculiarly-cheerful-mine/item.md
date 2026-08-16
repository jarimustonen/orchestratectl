---
created: 2026-08-06
updated: 2026-08-16
type: improvement
status: duplicate
priority: normal
labels: [defer-0.2.1]
closed: 2026-08-16
closed_by: claude
---

# orchestrate driver heartbeat for broader stall/liveness detection

## Description

Follow-up from `peculiarly-muddled-caption` (the read-time `stalled` hint for undriven orchestrate drivers).

## Scope of the shipped fix
`run show` / `run list` now flag a `--kind orchestrate` run as `stalled` when its driver node `n-0001` is still `pending` with **zero children** and no node-touching events past a 12-minute grace window — the exact "created but never driven" zombie.

## What it deliberately does NOT catch (this issue)
The read-time hint cannot detect these stalled shapes, because it has no orchestrator liveness signal:

1. **Driver spawned ≥1 child then the orchestrator died** — `children` is non-empty, so `stalled` stays false forever.
2. **Driver transitioned to `running` then stopped emitting events** — `is_stalled` only considers a `pending` driver node.
3. **All children terminal but the driver never rolled the run up.**
4. **Driver `blocked` indefinitely.**

## Proposed direction (from the llm-review of the parent fix)
A real fix needs an explicit orchestrator ownership/lease signal projected through the existing locked event path:
- emit a periodic `driver.heartbeat` (or lease-renewal) event while the orchestrator agent is actively driving;
- project `last_driver_heartbeat_at` onto the driver node;
- define "stalled" as a missed lease deadline (e.g. 3 missed heartbeats) regardless of children count or node status;
- this then generalizes the current narrow read-time heuristic to all four shapes above.

## Also worth considering (raised in review, deferred)
- Make the stall grace window configurable (env/config) rather than a hard-coded constant.
- `run list --stalled` filter to select suspected-stalled runs directly.
- Emitting `stalled_since` (a timestamp) rather than a bare bool so tooling can grade severity.

These must go through `LockedRun` + `append_and_apply_*` (state-integrity invariants 1-2) since they persist new state — unlike the parent fix, which was purely read-time.

## Decisions

### 2026-08-13T11:10:30Z · @adr-decision-2

DEFER-to-0.2.1: An explicit driver heartbeat/lease is the protocol path itself — deferred with it. The clean answer is the pi.dev self-report/lease plugin (0.2.1), not the 0.2.0 thin core. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).

## Resolution

### 2026-08-16T15:33:38Z · @claude

Duplikaatti: @supervisor-stall-detection. Sama puuttuva signaali (orkestroijan/agentin elonmerkki tapahtumahiljaisuuden aikana), eri kulma. Ratkaisu on yksi: hiljaisuuteen perustuva stall-havainto, ei kaksi rinnakkaista mekanismia. Ohjaimen heartbeat-tarve on kirjattu kohde-issuen alle.
