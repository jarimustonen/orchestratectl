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

## Comments

### 2026-06-27T18:09:41Z · @claude

Implemented option (b): LogGuard newtype + process-global flush_logs() hook.

- cli.rs: WorkerGuard wrapped in LogGuard (Arc<Mutex<Option<WorkerGuard>>>), reachable both from run()'s stack binding (Drop drains on normal unwinding) and the LOG_FLUSH OnceLock (flush_logs drains on process::exit paths). drain_cell is idempotent + poisoned-lock safe. Note: tracing-appender 0.2 NonBlocking::flush is a no-op; the only drain is WorkerGuard::drop sending Shutdown — so flush == drop the guard.
- tail.rs: flush_and_exit emits a diagnostic line then calls cli::flush_logs() before process::exit.
- Tests: deterministic unit test (slow-sink, fails without the fix) + end-to-end integration smoke test.

AUDIT of other process::exit sites (out of scope, fixed only event tail's per task):
- supervise/mod.rs:520 — signal-terminated supervisor exits 130/143 via process::exit, also bypassing the guard's Drop. It already flushes stdout but NOT the non-blocking tracing channel, so the supervisor's own buffered diagnostic logs near signal exit are lost the same way. Candidate follow-up: call cli::flush_logs() there too (flush_logs is already global, so it's a one-liner). Supervisor logs are arguably more valuable than tail's, so worth a spin-off.
