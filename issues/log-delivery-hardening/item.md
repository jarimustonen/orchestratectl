---
created: 2026-06-27
updated: 2026-06-28
type: improvement
reporter: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff]
---

# Harden log delivery: back-pressure or dropped-event accounting

_Source: crates/octl-cli/src/cli.rs_

## Description

init_logging (issue @nonblocking-log-appender) runs the non-blocking writer in lossy mode with a 128K-line buffer: under sustained back-pressure (disk cannot keep up) new events — including error!/warn! — are dropped silently with no counter surfaced. Accepted for MVP (favours supervisor responsiveness), but for an audit log this is a real gap. Post-MVP options: switch to NonBlockingBuilder::lossy(false) for back-pressure, and/or surface NonBlocking::error_counter() (dropped-event count) in the success-envelope warnings on shutdown. Also missing test coverage for the supervisor's intended high-volume path: a buffer-overflow/drop test and a SIGINT/SIGTERM-flush test for the long-lived supervise loop (supervise/mod.rs already exits its loop cooperatively on signal, so a clean-flush assertion is feasible).
