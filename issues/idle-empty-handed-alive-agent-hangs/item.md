---
created: 2026-07-28
updated: 2026-07-28
type: improvement
reporter: jari
status: open
priority: normal
related: ['@agent-skips-run-merge-idle-pending']
---

# Empty-handed idle alive agent still hangs pending

_Source: orchestratectl supervise (idle-unmerged review follow-up)_

## Description

Spun off from `/llm-review` of the idle-unmerged safety net (`agent-skips-run-merge-idle-pending`).

The idle-unmerged net terminalizes an autonomous agent that committed work (≥1 commit ahead of source) but went idle without `run merge`. It deliberately DECLINES the empty-handed case: an autonomous agent that committed NOTHING (0 commits ahead of source), left a clean worktree, and dropped to an idle shell — `node_recoverability` returns `None`, so `node_idle_unmerged` returns `None` and the run keeps hanging `pending` forever, same resource leak (live supervisor + tmux window + worktree).

This is a DISTINCT failure mode from committed-but-unmerged: there is nothing to salvage, and it overlaps the bounded-auto-retry path (which currently only fires for DEAD empty-handed agents, not ALIVE idle ones). Decide: terminalize as a plain `failed` (agent produced nothing and went idle), or reuse/extend the empty-handed retry path for the ALIVE case, guarded by the same three-clock idle signal so a still-working pre-first-commit agent is never tripped.

Acceptance: an ALIVE autonomous agent with 0 commits, clean worktree, all three activity clocks idle past threshold no longer hangs `pending`; a still-working pre-commit agent (CPU active or pane active) is not tripped; regression test.

## Reproduction evidence (2026-08-06, 3dbear-monorepo /stint-sessio)
Osui **kahdesti** samaan tehtävään reaalityössä, juuri kuten issue kuvaa (alive supervisor,
0 committia, idle, hangs `pending` ikuisesti):
- Tehtävä: `bcf-tool` pyright-warningien korjaus **2201-rivisessä** `bcf_validator.py`:ssä
  (~50 hajallaan olevaa `reportImplicitStringConcatenation`-kohtaa).
- Yritys 1 (`/worktree-spinoff`, normaali briiffi): supervisor elossa **~2 h**, 0 committia,
  0 uncommittoitua muutosta, issue jäi `open`iksi (agentti ei ehtinyt edes `in-progress`iin).
- Yritys 2 (`/worktree-spinoff`, tarkoituksella kevyt briiffi "suora mekaaninen fix, ei
  /llm-review, älä lue koko tiedostoa per edit"): sama **~40 min**, 0 committia.
- Molemmat jouduttiin `run cancel` + manuaalinen worktree-siivous. Kolmas yritys
  `/worktree-code`-interaktiivisena (ihminen ajoi) landasi ongelmitta.
- **Hypoteesi:** iso tiedosto + monta hajautettua editiä → agentti loop/stall per-edit
  full-reread-kuviossa. Empty-handed idle net (tämä issue) olisi napannut molemmat
  hukkaan menneet tunnit ~thresholdin kohdalla resurssivuodon estäen.
