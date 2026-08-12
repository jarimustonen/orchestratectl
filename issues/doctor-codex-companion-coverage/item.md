---
created: 2026-08-11
updated: 2026-08-12
type: improvement
status: done
priority: normal
closed: 2026-08-12
---

# doctor + prune do not cover codex skills or _shared companions

## Description

## Description

`doctor`'s skill.sync/orphan checks and `skill install --force` pruning are claude-layout only — they resolve paths via `claude_default_path` / `claude_skills_root`. Codex skills (`~/.codex/prompts/<name>.md`) and the new codex companion layout (`~/.codex/prompts/_shared/<filename>`, added by skill-companion-codex-layout) are never audited, version-checked, or pruned.

Surfaced by the /llm-review of skill-companion-codex-layout (4-model consensus). Consequences:
- A stale/missing codex companion in `_shared/` is invisible to `doctor` (claude companion has a `skill.sync.<name>.<file>` sub-check; codex has none).
- Removing a bundled skill leaves an orphaned codex prompt + possibly an orphaned `_shared/` companion with no prune path.
- No provenance marker on codex installs, so safe pruning has nothing to key on.

## Scope / decisions
- Extend the doctor skill.sync + orphan checks to the codex layout (or add a codex-specific check), including `_shared/` companions.
- Decide `_shared/` companion lifecycle: refcount by remaining codex skills whose rendered body references it, and prune only when the last referrer is removed.
- Provenance for codex (`~/.codex/prompts/` is flat and possibly shared across projects) — marker mechanism TBD.

Deferred like its parent: claude is the primary agent; codex is a secondary export.
