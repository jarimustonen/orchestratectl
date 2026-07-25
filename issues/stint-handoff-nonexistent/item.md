---
created: 2026-07-25
updated: 2026-07-25
type: bug
status: fixed
priority: normal
closed: 2026-07-25
---

The stint `SKILL.template.md` references a `/handoff` slash-command skill in its
frontmatter description, Phase 7, and the NOT-for list. No such skill exists — only
`/wrap-up`. Every agent reading `stint` assumes a separate `/handoff` skill that isn't
there.

**Fix:** describe the handoff step as an inline action ("update the TODO.md handoff
block"), never as a `/handoff` skill call, consistently across the frontmatter
description, Phase 7, and the NOT-for list.

Behavior stays identical: (1) update the TODO.md handoff block so a fresh agent can
resume, (2) commit that TODO.md update immediately as its own commit, (3) then run
`/wrap-up`. Only the fictional skill name goes away.
