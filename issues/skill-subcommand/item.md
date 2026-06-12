---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# skill subcommand (installer for companion skills)

## Description

orchestratectl skill list|show|install — companion-skill installer per AGENTS-AI-FIRST-CLI §15. Skill files live under crates/octl-cli/skills/ and ship with the binary. MVP ships the subcommand + mechanics + 2 seed skills (octl-run-overview, octl-spawn-spinoff). Full skill library (replacing /worktree-*) is post-MVP. Cheap; can land any time after scaffolding. **Depends on** cargo-scaffolding only.
