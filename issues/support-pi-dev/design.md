# Design — pi.dev skill install copies companion files

## Root cause

`cmd_install` mirrors each skill's `SKILL.md` into the pi.dev per-skill dir
(`~/.pi/agent/skills/<name>/SKILL.md`) but the original "vendored filter"
mirrored **only** `SKILL.md`, never companion resources. A skill whose body
links to a sibling companion (`stint-start` → `AGENTS-EXECUTION-DAG.md`) then
has a dangling link in its pi copy, and `/stint-start`, which is instructed to
STOP if the companion is missing, aborts with ENOENT before Phase 0.

Key layout fact: **pi uses per-skill directories, exactly like claude** (not a
flat prompts dir like codex). So a companion installed as a plain sibling of the
pi `SKILL.md` resolves the body's `](AGENTS-EXECUTION-DAG.md)` link (and
`stint-handoff`'s cross-skill `](../stint-start/AGENTS-EXECUTION-DAG.md)`) with
**no link rewrite** — byte-identical bodies and companions, same as claude.

## Fix

Reverse the vendored filter for pi: install companion resources beside the pi
`SKILL.md`, matching the claude layout. Extend the pi mirror's out-of-band
provenance (`state/pi-installed-skills.json`) and `doctor` to cover companions
symmetric with claude/codex:

1. **Install** — the pi plan block pushes one `PlanItem` per companion (agent
   `"pi"`, sibling path) in addition to the `SKILL.md` item. Preflight already
   treats every `agent == "pi"` item uniformly (skip-if-present-no-force,
   overwrite-under-force), so companions inherit the same safe divergence rules.
2. **Provenance** — `PiSkillRecord` gains `companions: { filename -> sha256 }`
   (serde-default, backward compatible; schema stays v1). Each pi companion
   write is filed under its owning skill.
3. **Prune** — pruning a de-registered pi skill now removes its recorded
   companions (each verified to still hash to what we wrote; a diverged/user
   copy is left) before the non-recursive `remove_dir`, so the per-skill dir is
   cleaned rather than left dangling.
4. **Doctor** — `check_pi` adds `skill.sync.<name>.pi.<file>` (forward companion
   drift vs the bundled body — advisory, no autonomous fix, like the codex
   companion check) and `skill.orphan.<name>.pi.<file>` (a recorded companion
   the binary no longer bundles). Both gated on the provenance record, so a
   pi-less host stays 0-check / 0-warn.

## Post-review hardening (`/llm-review` + `/assess-findings`)

A 4-model review (`history/review-support-pi-dev.md`) + triage
(`history/assessment-support-pi-dev.{json,md}`) produced 9 FIX / 1 SPIN-OFF / 3
DROP. Applied in this PR:

- **F1** (consensus) — a still-registered skill that sheds a companion is now
  reconciled on `--force` (`reconcile_pi_companions`): the orphan file is removed
  (our unmodified copy only) and dropped from the record, so the
  `skill.orphan.<name>.pi.<file>` doctor warning is actually fixable rather than a
  permanent loop.
- **F2** — `PI_PROVENANCE_SCHEMA_VERSION` bumped to **2**, so an older binary
  refuses the new record (fail-closed) instead of silently dropping the
  `companions` field on rollback.
- **F3/F6** — `prune_pi_mirror_at` now prunes companions BEFORE the `SKILL.md`,
  defers (`Kept`) when a companion delete fails, cleans companions even when the
  `SKILL.md` is already absent, and narrates a non-regular leftover.
- **F4** — an unrecorded pi-companion owner now warns (`pi_companion_unrecorded`)
  instead of silently dropping the write.
- **F5** — pi prune + doctor gained the case-insensitive registered-name guard the
  claude path has (APFS safety).
- **F7/F8/F13** — doctor surfaces corrupt record filenames, binds
  `companion_sources` once, and `is_simple_skill_name` documents its filename reuse.

SPIN-OFF **F11** → issue `pi-provenance-flat-file-model` (flat per-file provenance
redesign). DROPPED: F9 (accurate warning), F10 (self-inflicted symlink, pre-existing),
F12 (cosmetic drift signal).

## Non-goals / accepted

- pi companion drift carries **no** autonomous `FixAction` — the applier runs
  `skill install <name> --force`, which dual-homes and would force-overwrite the
  claude copy too (symmetric with the pi SKILL.md and codex companion arms).
- Record read-modify-write stays unlocked (parity with the claude/codex/pi
  markers); mutation commands are not meant to run concurrently.
