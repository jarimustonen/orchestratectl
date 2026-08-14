---
created: 2026-08-13
updated: 2026-08-14
type: improvement
status: done
priority: normal
closed: 2026-08-14
closed_by: pi-dev-mirror-flat-worktree
commits:
- hash: 48ddf73
  summary: flat per-file pi provenance model (v3)
- hash: c99a1d0
  summary: apply /llm-review findings
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

## Resolution

### 2026-08-14T04:59:03Z · @pi-dev-mirror-flat-worktree

Flat per-file pi provenance (v3): PiSkillRecord{cli_version, files:{relpath:{sha256,kind}}}; v1/v2->v3 read/upgrade path via RawPiSkillRecord serde(from) with empty-hash + SKILL.md-alias guards; strict future-schema fail-closed retained. PlanKind replaces pi_companion_of inference; flat per-file prune/reconcile (companions-first, per-file relinquish/retry, NotFound-vs-transient distinction). Doctor coverage preserved (PiManagedSkill.sha256->Option for companion-only records). Green gate + insta snapshots + deploy-verify (clean install and live v2->v3 upgrade both doctor 0/0). 3-model /llm-review triaged in history/review-pi-provenance-flat-file-model.md.
