---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: done
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff, cargo-scaffolding-review]
closed: 2026-06-27
commits:
- hash: df77602
  summary: non-blocking JSONL log writer
- hash: e78d056
  summary: apply llm-review findings
---

# tracing_appender::non_blocking for JSONL log throughput

## Description

Current init_logging writes synchronously per event. At supervisor polling rates (500ms ticks across ~100 nodes) this becomes a bottleneck and can interleave bytes >PIPE_BUF. Switch to tracing_appender::non_blocking with a background writer thread. Surfaced by cargo-scaffolding review.
