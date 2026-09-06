---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: fixed
priority: normal
closed: 2026-08-04
---

# skill install leaves de-registered bundled skills stranded in ~/.claude/skills

_Source: crates/taskfleet-cli/src/skill.rs_

## Description

Surfaced live during the /stint → stint-start + stint-handoff split (2026-08-04). After the bundled 'stint' skill was removed from the registry and 'taskfleet skill install --force' was run, the two new skills installed correctly BUT the de-registered '~/.claude/skills/stint/' directory was left behind (the old monolith SKILL.md). 'taskfleet doctor' did NOT flag it (it only checks skill.sync.* for REGISTERED skills, not orphans). Impact: a renamed/removed bundled skill keeps showing up as an available slash-command pointing at stale instructions — exactly the drift the bundling is meant to prevent. Had to 'rm -rf ~/.claude/skills/stint' by hand.

Fix options: (a) 'skill install' prunes install-dir entries that were installed by taskfleet but are no longer in the registry (needs a provenance marker so we never delete a user's own hand-authored skill of the same name); (b) 'doctor' adds an orphan check (skill.orphan.<name>) that warns when ~/.claude/skills/<name> looks taskfleet-installed but is de-registered; (c) both. Prefer a provenance marker (e.g. a managed-by-taskfleet stamp) so pruning is safe.
