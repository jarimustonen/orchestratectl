---
created: 2026-06-27
updated: 2026-06-27
type: bug
reporter: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff]
---

# event tail process::exit bypasses non-blocking log guard flush

_Source: crates/octl-cli/src/event/tail.rs_

## Description

After switching init_logging to tracing_appender::non_blocking (issue @nonblocking-log-appender), the WorkerGuard held in cli::run() flushes buffered log events only on normal stack unwinding. event tail's flush_and_exit() calls std::process::exit() (tail.rs:414), which bypasses Drop — so any of this process's own tracing events still buffered in the non-blocking channel are silently lost on exit. With the previous synchronous writer this was harmless (each event hit disk immediately). Narrow: only affects the tail command's own diagnostic logs, not the followed event stream. Fix options: (a) thread an ExitCode back up through run() instead of process::exit, or (b) expose a flush hook the guard owner can call before exit. Also add a regression test asserting tail's startup log line reaches disk.
