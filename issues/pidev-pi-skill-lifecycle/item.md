---
created: 2026-08-11
updated: 2026-08-12
type: improvement
status: done
priority: normal
closed: 2026-08-12
commits:
- hash: 8943e2f
  summary: pi mirror lifecycle — provenance record + prune + doctor drift
---

# Pi skill lifecycle: prune orphans + doctor drift check via out-of-band provenance

## Description

# Pi skill lifecycle: prune orphans + doctor drift check via out-of-band provenance

## Problem

`orchestratectl skill install` now dual-homes each skill's `SKILL.md` into
`~/.pi/agent/skills/<name>/` (issue `pidev-dual-home-skills`). That mirror has
**no lifecycle management**:

- **Orphans never pruned.** When a skill is de-registered from the catalog, the
  claude copy is pruned (`prune_eligible` scans `claude_skills_root()` only) but
  the pi mirror at `~/.pi/agent/skills/<old-name>/` stays forever. pi.dev keeps
  surfacing a stale `/skill:<old-name>`.
- **Doctor blind to pi.** `doctor`'s `skill.sync` / `skill.orphan` checks are
  claude-only, so a stale or divergent pi copy (a second source of truth) has no
  drift detection.

## Constraint that blocks the naive fix

`pidev-dual-home-skills` deliberately forbids the `.orchestratectl-managed`
marker in the pi dir. Without a provenance signal we **cannot** distinguish an
orchestratectl-left-behind pi dir from a user's own hand-authored pi skill — so a
naive "pi orphan" warning would false-positive on every user skill. This is why
orphan handling was deferred rather than bolted on.

## Ask

Design an **out-of-band provenance record** (e.g.
`~/.orchestratectl/state/pi-installed-skills.json`, recording names + content
hashes orchestratectl wrote to pi), then build on it:

1. Prune pi mirrors of de-registered skills, gated on `--force` like the claude
   prune, but keyed on the provenance record (never touch dirs we didn't write).
2. A `doctor` `skill.sync.<name>.pi` drift check mirroring the claude one.
3. Careful symlink-escape / atomic-write handling (same rigor as
   `managed_orphan_dirs`).

## Context

Filed from `/assess-findings` triage of the dual-home review
(`history/assessment-dual-home-skills.md`, finding F2 — SPIN-OFF). Confirmed by a
4-model `/llm-review` panel as real but out-of-scope for the minimal dual-home
change.
