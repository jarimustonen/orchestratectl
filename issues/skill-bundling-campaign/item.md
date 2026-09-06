---
created: 2026-06-28
updated: 2026-06-29
type: epic
owner: jari
closed: 2026-06-29
status: done
priority: high
epic: taskfleet-mvp
---

# Skill-bundling campaign: replace homebase /worktree-* + /orchestrate + /fan-out via binary-bundled SKILL.md

## Description

Per AGENTS-AI-FIRST-CLI §17 (binary is source of truth, skill follows): author the full skill family as bundled SKILL.md files in `crates/taskfleet-cli/skills/`, install via `taskfleet skill install --force` to `~/.claude/skills/`. Replaces homebase versions. Sequential authoring so each completed skill set the contract for the next.

## Outcome

All 10 planned phases shipped, plus a bonus 11th (`worktree-merge`) bundled mid-campaign when an interactive-worktree gap surfaced live. Binary ships **13 skills**:

- `taskfleet-overview`, `taskfleet-run-overview`, `taskfleet-spawn-spinoff`
- `worktree-code`, `worktree-spinoff`, `worktree-merge`
- `worktree-research`, `worktree-bugfix`, `worktree-technical-decision`, `worktree-make-skill`
- `worktree-orchestrated`, `fan-out`, `orchestrate`

`taskfleet doctor` reports 63 ok / 0 fail. `~/.claude/skills/` deployed and end-to-end loops proven:

- `/worktree-spinoff` (autonomous) — spawn → work → merge → self-cleanup with zero manual intervention.
- `/worktree-code` + `/worktree-merge` (interactive) — same loop, human-gated merge.
- `/orchestrate` smoke-tested with a 3-feature DAG; works end-to-end.

## Follow-ons

The `/orchestrate` smoke surfaced 4 polish bugs, now tracked separately and gating publication:

- [`headless-parent-session-rejected`](../headless-parent-session-rejected/item.md)
- [`orchestrated-source-branch-ignored`](../orchestrated-source-branch-ignored/item.md)
- [`failed-spawn-leaves-phantom-child`](../failed-spawn-leaves-phantom-child/item.md)
- [`supervisor-worktree-remove-no-force`](../supervisor-worktree-remove-no-force/item.md)

These belong to the pre-publication campaign tracked in `TODO.md`, not to this epic.
