---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
---

# tracing_appender::non_blocking for JSONL log throughput

## Description

Current init_logging writes synchronously per event. At supervisor polling rates (500ms ticks across ~100 nodes) this becomes a bottleneck and can interleave bytes >PIPE_BUF. Switch to tracing_appender::non_blocking with a background writer thread. Surfaced by cargo-scaffolding review.
