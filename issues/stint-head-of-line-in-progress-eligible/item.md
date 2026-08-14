---
created: 2026-08-14
updated: 2026-08-14
type: task
status: open
priority: normal
labels: [skill]
---

# stint/orchestrate head-of-line: in-progress issues should be eligible (resumable), not excluded

## Description

## Comments

### 2026-08-14T03:42:36Z · @jari

Skill-layer follow-up to issuectl's dag-inprogress-is-spawnable (issuectl repo). The stint/orchestrate execution-DAG convention currently says the head-of-line is 'eligible iff … NOT already in-progress' (~/.claude/skills/stint-start/SKILL.md and AGENTS-EXECUTION-DAG.md; the /orchestrate skill shares the spawnable notion). That must be realigned to the corrected model: in-progress ≠ 'being worked right now' — it means STARTED, not done. The DAG is consulted only when nothing is actively running, so an in-progress issue is a RESUMABLE candidate that should be surfaced (aggressively), not excluded. Double-work prevention is the caller's reservation/claim responsibility. Update the head-of-line eligibility wording + the reserve-at-launch guidance accordingly. Decided 2026-08-13 with @jari during an issuectl stint.
