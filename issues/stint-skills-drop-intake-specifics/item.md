---
created: 2026-08-13
updated: 2026-08-13
type: improvement
status: open
priority: normal
related: ['@stint-handoff-intake-check', '@stint-start-autonomous']
---

# Remove project-specific intake concepts leaked into stint-handoff + execution-DAG (keep these skills generic/open-source)

_Source: crates/octl-cli/skills/stint-handoff, crates/octl-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md_

## Description

`/stint-start` and `/stint-handoff` are shipped as generic, open-source skills that must
carry ZERO downstream-project-specific concepts. The `stint-handoff-intake-check` +
`stint-start-autonomous` work (commits ~148ac4b, ~3a1a033) violated that: it baked a
particular personal setup's bug-intake vocabulary into these skills and into the shared
`AGENTS-EXECUTION-DAG.md` reference. That coupling must come out; the intake behaviour
moves to the CONSUMING project's own personal skill layer (`/wrap-up` → `/triage-bugs`
there), leaving these skills generic.

## Leaked specifics to remove / generalize
- `crates/octl-cli/skills/stint-handoff/SKILL.template.md` — the intake-check step
  references specific labels (`via:telegram`, `needs-triage`), a specific intake tool,
  and specific slug schemas (`tg-bug-*`, `intake-bug-<repo>-<hash>`). None of these are
  generic. Remove the intake-specific step.
- `crates/octl-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md` — the DAG active-set now
  EXCLUDES a specific `needs-triage` label (plus the matching `comm -3` drift jq filter +
  eligibility rule). That label is not a universal issuectl status — it is downstream
  project vocabulary. Revert the exclusion; the generic active set is non-terminal minus
  the generic `deferred` notion only.
- `crates/octl-cli/skills/stint-start/SKILL.template.md` — scrub intake framing
  ("consume the handoff-prepared intake", etc.). The autonomy tightening itself is
  generic and should STAY; only the intake wording goes.
- CHANGELOG `[Unreleased]` entry — rewrite to describe only the generic autonomy change,
  dropping the intake-check description.

## What the handoff MAY keep (generic only)
At most an ABSTRACT, vocabulary-free nudge — "are there new/unscheduled issues in the
tracker not yet in the DAG?" via a generic `issuectl` query — with no notion of intake,
telegram, labels, or any tool. Optional; a full revert to the pre-feature handoff is also
acceptable. It must not know WHERE issues came from.

## Why now = issue only (not a PR)
A larger architecture refactor is in flight in this repo; do NOT land a competing code
change against it. File-and-hold: reconcile this cleanup with that refactor when it
settles. Correctness does not depend on it — the downstream personal layer handles intake
on its own against the released generic skills.

## Context
Design + downstream side live in the consuming project (epic `stint-management-layer`,
issues `stint-intake-lifecycle`, `triage-bugs-handoff-reconcile`, and the `/wrap-up`↔
`/triage-bugs` wiring). Supersedes the intake half of `stint-handoff-intake-check`.
