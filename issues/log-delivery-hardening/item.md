---
created: 2026-06-27
updated: 2026-06-28
type: improvement
reporter: jari
status: done
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff]
closed: 2026-06-28
---

# Harden log delivery: back-pressure or dropped-event accounting

_Source: crates/taskfleet-cli/src/cli.rs_

## Description

init_logging (issue @nonblocking-log-appender) runs the non-blocking writer in lossy mode with a 128K-line buffer: under sustained back-pressure (disk cannot keep up) new events — including error!/warn! — are dropped silently with no counter surfaced. Accepted for MVP (favours supervisor responsiveness), but for an audit log this is a real gap. Post-MVP options: switch to NonBlockingBuilder::lossy(false) for back-pressure, and/or surface NonBlocking::error_counter() (dropped-event count) in the success-envelope warnings on shutdown. Also missing test coverage for the supervisor's intended high-volume path: a buffer-overflow/drop test and a SIGINT/SIGTERM-flush test for the long-lived supervise loop (supervise/mod.rs already exits its loop cooperatively on signal, so a clean-flush assertion is feasible).
