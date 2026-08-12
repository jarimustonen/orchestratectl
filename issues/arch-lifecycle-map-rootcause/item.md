---
created: 2026-08-12
updated: 2026-08-12
type: task
status: done
priority: high
epic: lifecycle-architecture-review
closed: 2026-08-12
---

# Map the lifecycle subsystem + bug taxonomy + shared root cause

## Description

PHASE 1 (evidence base). Map the run/supervisor/agent lifecycle end-to-end (supervise/*, run/*, reducer/lock/events, watchdog, merge/teardown, notify). Produce: (a) an architecture map of the subsystem and its state model; (b) a taxonomy of the ~24 cluster-A/B open issues grouped by the (pid × pane × branch × report) signal-combination they represent; (c) a root-cause writeup: inference-by-polling vs protocol-based self-reporting, and which bugs are ESSENTIAL complexity vs ACCIDENTAL. Deliverable: sourced markdown under issues/lifecycle-architecture-review/analysis.md. Read-only investigation, no code changes.
