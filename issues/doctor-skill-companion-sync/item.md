---
created: 2026-08-04
updated: 2026-08-04
type: improvement
status: open
priority: normal
related: ['@split-stint-start-handoff']
---

# doctor skill.sync should also check companion resource files

## Description

The `skill.sync.<name>` doctor check only validates each bundled skill's SKILL.md cli_version. Bundled skills can now ship companion resource files (e.g. `stint-start/AGENTS-EXECUTION-DAG.md`, installed as a sibling of SKILL.md for the claude agent). A companion that is missing, stale, or user-edited leaves the skill's in-body link broken while doctor still reports the skill as in-sync. Add a sub-check that verifies every declared companion resource is present at its expected install path and version-synced (both reviewers of split-stint-start-handoff flagged this). Low urgency: the current common flows install both files together, and the skills carry a runtime 'stop if the reference is missing' guard.
