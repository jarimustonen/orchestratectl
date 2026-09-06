---
created: 2026-08-12
updated: 2026-08-12
type: task
status: done
priority: high
epic: lifecycle-architecture-review
closed: 2026-08-12
---

# Feature-usage / dead-weight drag audit

## Description

PHASE 1 (parallel). Inventory every user-facing surface — the run kinds (code, spinoff, orchestrated, research, technical-decision, make-skill, bugfix, fan-out, orchestrate), the code-pipeline/wave-build subsystem, harness bakeoff/multi-harness, discussions, spinoffs — and classify each as ACTIVELY USED vs MAINTAINED-BUT-IDLE, with the maintenance-drag cost (edge cases, supervisor branches, tests) each imposes. Goal: find complexity that exists to serve capabilities nobody uses. Deliverable: a drag inventory in issues/lifecycle-architecture-review/feature-audit.md. Read-only.

## Comments

### 2026-08-12T05:51:17Z · @jari

STEER (2026-08-12, primary user): actual usage is NARROW — 'we have quite limited use cases; it's very possible some options really aren't needed.' Ground the audit in Jari's REAL use set (grep the stint/worktree skill call patterns + ~/.taskfleet/runs history + confirm WITH Jari), then flag every unused kind/flag/subsystem as a removal candidate with its drag cost. BIAS TOWARD CUTTING. Suspects: the 9 run-kinds, code-pipeline/wave-build, harness bakeoff, discussions/spinoff-proposals. Feeds DECISION-1.

### 2026-08-12T06:50:02Z · @agent-arch-audit

Audit complete. Deliverable: issues/lifecycle-architecture-review/feature-audit.md — drag inventory of all 9 run-kinds, pipeline/floor/harness subsystems, harness selection, discussions, spin-off proposals. Key findings: 83% of 717 runs are spinoff (96% of recent 120); bugfix never run; --harness invoked by 0 skills and 0/717 runs; pipeline+harness referenced by 0 workflow skills (~30k idle LOC); discussions 4/717, spin-off proposals 5/717. 11 removal candidates (R1-R11) + CONFIRM-WITH-JARI section for DECISION-1.

