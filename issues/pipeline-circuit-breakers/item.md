---
created: 2026-07-25
updated: 2026-07-26
type: task
status: in-progress
priority: normal
related: ['@pipeline-walking-skeleton']
---

# Pipeline circuit-breakers + cost/token metering (T6)

## Description

The T5 skeleton has no resource ceilings. Add the design §9 supervisor-owned, deterministic circuit-breakers that force ESCALATE/abort regardless of convergence: cost/token ceiling (target ≤ ~2× all-Opus) with a kill-switch, wall-time, process-count, storage, and a repeated-identical-failure breaker. Requires per-node cost instrumentation (the harness already surfaces Usage from --output-format json; wire it into a per-run spend tally). Multi-round convergence bounding depends on this.
