---
created: 2026-08-14
updated: 2026-08-14
type: task
status: done
priority: high
labels: [skill]
closed: 2026-08-14
---

# stint/orchestrate head-of-line: in-progress issues should be eligible (resumable), not excluded

## Description

## Comments

### 2026-08-14T03:42:36Z · @jari

Skill-layer follow-up to issuectl's dag-inprogress-is-spawnable (issuectl repo). The stint/orchestrate execution-DAG convention currently says the head-of-line is 'eligible iff … NOT already in-progress' (~/.claude/skills/stint-start/SKILL.md and AGENTS-EXECUTION-DAG.md; the /orchestrate skill shares the spawnable notion). That must be realigned to the corrected model: in-progress ≠ 'being worked right now' — it means STARTED, not done. The DAG is consulted only when nothing is actively running, so an in-progress issue is a RESUMABLE candidate that should be surfaced (aggressively), not excluded. Double-work prevention is the caller's reservation/claim responsibility. Update the head-of-line eligibility wording + the reserve-at-launch guidance accordingly. Decided 2026-08-13 with @jari during an issuectl stint.

### 2026-08-14T05:20:38Z · @stint-orchestrator

Jari (2026-08-14): important — MUST land in 0.2.0. Prioritised to high; spawning a worktree this round.

## Resolution

### 2026-08-14T05:57:31Z · @issuectl

Realigned head-of-line eligibility across the three source bundled skills: dropped 'not already in-progress' from the eligibility predicate; in-progress now means STARTED (resumable, surfaced aggressively), not excluded; double-work prevention stated as the caller's reserve-at-launch/claim responsibility (launched-but-unsettled run this round holds its issue + collision files). Files: stint-start/AGENTS-EXECUTION-DAG.md (canonical head-of-line + spawnable rule), stint-start/SKILL.template.md (summary + Phase 2 reserve guard), orchestrate/SKILL.template.md (shared spawnable-feature notion). /llm-skill-review (gemini+gpt-5.6, executor+cross-skill lenses) ran; applied all confirmed findings (removed the false 'DAG only consulted when nothing running' premise that would serialize parallel dispatch; removed an idempotency-resume over-claim contradicting the -r2 retry recovery; standardized on 'launched-but-unsettled' vocab). Green gate: fmt/clippy/test all green (one supervise timing flake, passes in isolation). Mirrors issuectl dag-inprogress-is-spawnable.
