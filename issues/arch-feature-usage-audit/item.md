---
created: 2026-08-12
updated: 2026-08-12
type: task
status: open
priority: high
epic: lifecycle-architecture-review
---

# Feature-usage / dead-weight drag audit

## Description

PHASE 1 (parallel). Inventory every user-facing surface — the run kinds (code, spinoff, orchestrated, research, technical-decision, make-skill, bugfix, fan-out, orchestrate), the code-pipeline/wave-build subsystem, harness bakeoff/multi-harness, discussions, spinoffs — and classify each as ACTIVELY USED vs MAINTAINED-BUT-IDLE, with the maintenance-drag cost (edge cases, supervisor branches, tests) each imposes. Goal: find complexity that exists to serve capabilities nobody uses. Deliverable: a drag inventory in issues/lifecycle-architecture-review/feature-audit.md. Read-only.
