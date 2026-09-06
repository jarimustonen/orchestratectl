---
created: 2026-08-16
updated: 2026-08-17
type: task
status: done
priority: high
related: ['@stint-skills-drop-intake-specifics']
lane: skills
lane_seq: 2
closed: 2026-08-17
---

# Migrate stint skills from TODO markdown DAG to issuectl dag

## Problem

The bundled `stint-start` and `stint-handoff` skills still require and parse `AGENTS-EXECUTION-DAG.md`, which defines the retired hand-maintained `TODO.md` execution-DAG block. The reference says that TODO owns lane order and `collision:` tags, requires `execution-dag:begin/end` delimiters, and calculates drift with `comm -3`. Consuming repositories have moved scheduling to issuectl frontmatter and `issuectl dag`.

## Required change

- Make `issuectl dag --json` the sole scheduling source in `stint-start` and `stint-handoff`. Read lane order, dependency state, collision tokens, computed heads, and spawnability from its JSON rather than from `TODO.md`.
- At launch, pass current live run holds to `issuectl dag --reservations` so spawnability accounts for in-flight lane and collision reservations.
- Delete `crates/taskfleet-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md` and remove the installation/runtime dependency on it.
- Remove all instructions to create, merge, parse, validate, or commit a markdown `## Execution DAG` block. `TODO.md` remains only a handoff narrative.
- Preserve generic operating-policy and safety guidance where it remains useful, but move it into the relevant skill templates or a newly named generic reference if shared prose is genuinely needed. Do not retain the retired DAG notation.
- Update bundled-skill install/doctor snapshots and tests.

## Acceptance criteria

A fresh `taskfleet skill install` no longer installs `AGENTS-EXECUTION-DAG.md`; neither stint skill mentions TODO markdown DAG delimiters, `comm -3`, `GLOBAL HEAD-OF-LINE`, or prose `collision:` tags; and the documented scheduling flow works solely through `issuectl dag --json` plus optional reservations.

## Context

Homebase issue `adopt-issuectl-dag` records the downstream migration and identified this bundled-skill cutover as the remaining work. This follows `stint-skills-drop-intake-specifics`, which removes separate project-specific legacy vocabulary from the same skills.
