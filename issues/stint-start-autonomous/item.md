---
created: 2026-08-12
updated: 2026-08-12
type: feature
status: in-progress
priority: normal
related: ['@stint-handoff-intake-check']
commits:
- hash: 3a1a033eview fixes
  summary: cold-start branch + autonomy hard-stops + lifecycle wording
---

# stint-start: maximally autonomous — no questions, resume straight from handoff-prepared state

_Source: skills/stint-start_

## Description

Pair of `stint-handoff-intake-check`. The stint flow should run as a smooth loop:
`/stint-handoff` (human-interaction + agenda build) → "klar" → `/stint-start` just
goes. Today `/stint-start` still has interactive/asking surfaces; Jari wants it
**maximally autonomous** — resume straight from the handoff-prepared state and start
executing, asking nothing it can decide or read for itself.

## Scope
Audit `/stint-start` for every place it pauses to ask the human and remove or
defer each where the handoff has already supplied the answer:
- Orient/plan phase: consume the `## 🔄 Continue here` block + DAG the handoff left,
  rather than re-deriving or re-confirming the plan with the user.
- Only genuinely-blocking ambiguity (something the handoff could NOT have resolved)
  may surface — and even then, prefer logging a best-judgment decision and
  proceeding (bold first, ask later), consistent with the autonomous-worktree ethos.
- Keep the product-owner status report to the user (that is output, not a question).

## Contract with the handoff
`stint-handoff-intake-check` is responsible for leaving the start fully prepared
(agenda + folded-in intake items). This issue makes `/stint-start` TRUST that
prepared state and run. The two ship together.

## Boundaries
- Do not remove the single deploy-ownership / worktree-spawn responsibilities — only
  the *human-questioning* surfaces.
- Generic across projects — reads specifics from the repo's own AGENTS.md/TODO.md.

## Cross-repo
Design home: homebase `issues/stint-intake-lifecycle` (epic `stint-management-layer`).
