---
created: 2026-08-13
updated: 2026-08-13
type: improvement
status: open
priority: normal
---

# Flat per-file provenance for the pi.dev skill mirror

## Description

Refactor the pi.dev skill-mirror provenance from a nested body-owns-companions
record to a flat per-file model. Surfaced by the /llm-review of `support-pi-dev`
(2/4 reviewers converged; see history/review-support-pi-dev.md and
history/assessment-support-pi-dev.md F11).

## Problem

`PiSkillRecord { sha256, cli_version, companions: {file->sha} }` nests companions
under the `SKILL.md` body record. The body becomes an artificial ownership root,
which is the structural root cause of several lifecycle edge cases the point-fixes
in the support-pi-dev PR had to patch individually:

- a companion written while the body write is preflight-skipped has no record to
  attach to (patched: emit a warning);
- de-registration prune couples companion cleanup to body removal / body-hash
  divergence (patched: reorder + return Kept-on-failure);
- a still-registered skill's dropped companion must be reconciled separately from
  the body (patched: dedicated reconciliation loop).

Each fix is correct but the model forces them.

## Proposed direction

- `PiSkillRecord { cli_version, files: BTreeMap<relpath, PiFileRecord> }` where
  `PiFileRecord { sha256, kind: Skill|Companion }` — every mirrored file tracked
  independently, so ownership/relinquish/retry decisions are per file.
- Replace `PlanItem { agent, path, content, pi_companion_of: Option<_> }` with a
  `PlanKind { Skill { name } | Companion { owner, filename } }` enum, removing the
  stringly-typed `agent == "pi"` + optional-field inference in the write loop.
- Rework prune + doctor to iterate the flat file map; drop the empty-dummy-body
  hashes the nested model needs.

## Constraints

- Record migration: read the current (post-support-pi-dev) schema and upgrade in
  place; keep the strict future-schema guard.
- Preserve every safety invariant (single-path-component validation, regular-file
  + hash check before delete, advisory-only doctor fixes).
- Symmetric parity with the claude/codex marker lifecycles.
