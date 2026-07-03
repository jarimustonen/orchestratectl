---
created: 2026-07-03
updated: 2026-07-03
type: bug
status: open
priority: normal
related: ['@supervisor-dead-merge-no-teardown']
---

# Verify recycled legacy bare-integer supervisor.pid via process identity

_Source: crates/octl-cli/src/run/merge.rs_

## Description

Documented KNOWN RESIDUAL from supervisor-dead-merge-no-teardown. A legacy supervisor.pid file without a start-time, whose pid was recycled by an unrelated live process, reads as 'alive', so run merge skips reattach and could strand the run. Modern start-time pid files are immune. Robust fix needs a process-identity check (e.g. ps command matches 'orchestratectl supervise <id>'). Rare — legacy files are a phased-out migration artifact — filed for completeness. Non-blocking for v0.1.0.
