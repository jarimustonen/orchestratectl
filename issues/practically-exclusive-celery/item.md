---
created: 2026-08-06
updated: 2026-08-06
type: task
status: open
priority: normal
---

# Meter agent usage spent before a wave-build worker panic

## Description


Follow-up from the `immoderately-dirty-cushion` review.

A wave-build worker wraps its build in `catch_unwind`; on a panic it returns
`WaveJob::Panicked(msg)` carrying no `Usage`. The agent invocations the worker
metered before the panic are therefore never folded into `run.meter`, so the §9
resource ceilings can under-count spend on a crashing build.

**Task:** meter partial usage even on the panic path — e.g. accumulate `Usage`
through a thread-safe channel / `Arc<Mutex<..>>` as each harness call returns
inside `build_chunk_in_wave`, rather than only returning it in the success value,
so a panic doesn't lose the tally. Low priority (panics are bugs, not steady
state), but it keeps the cost breaker honest.
