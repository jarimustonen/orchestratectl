---
created: 2026-08-12
updated: 2026-08-12
type: feature
status: in-progress
priority: normal
---

# stint-handoff: end-of-stint intake check — surface new bugs, fold approved into next-stint agenda

_Source: skills/stint-handoff_

## Description

The MVP surface of the intake lifecycle (homebase epic `stint-management-layer`,
child `stint-intake-lifecycle`). `/stint-handoff` is the point where the next
stint's agenda (TODO.md `## 🔄 Continue here` block + the execution DAG) is built
AND where the human is in the loop — so it is the right home for the "are there new
bugs?" gate that today never runs (the flow is only `/stint-start → /stint-handoff`,
so nobody invokes `/triage-bugs` and filed intake items pile up untriaged).

## Scope (MVP — deliberately thin)
Add a step to `/stint-handoff`, at the human-interaction point, that:
1. **Detects newly-arrived intake items** for THIS repo — filed bugs (intakectl
   drainer / `via:telegram`, `untriaged`) and, later, agent-authored cross-system
   reports. Query via issuectl (untriaged/needs-triage in `issues/`) — the repo's
   own queue is the source of truth.
2. **Lists them to the human** — LIGHT listing only (title + one-line + slug). NOT a
   deep per-item analysis; Jari asks interactively for more on any item he cares
   about. (Deep `/triage-bugs`-style analysis stays for the full lifecycle version.)
3. On the human's ack, **folds the chosen items into the next stint's agenda** — the
   `## 🔄 Continue here` handoff block and/or the DAG — so `/stint-start` picks them
   up with zero further input. Ideally a single "klar" ack promotes them.

## Boundaries / invariants
- Presentation + fold only; no silent auto-promotion (human gate, per
  `stint-intake-lifecycle`).
- Do NOT push gateway/collision logic into issuectl (epic invariant) — all logic is
  orchestrator-side; issuectl is queried, not extended.
- Empty queue ⇒ the step is a no-op and stays fast.
- Pairs with `stint-start-autonomous`: handoff must leave the start so complete the
  next `/stint-start` needs no questions.

## Cross-repo
Design home: homebase `issues/stint-intake-lifecycle` (epic `stint-management-layer`).
Related homebase: `wrapup-enqueue-intake`, `triage-bugs` skill. Generic across
projects — reads repo specifics from the project's own AGENTS.md/TODO.md.
