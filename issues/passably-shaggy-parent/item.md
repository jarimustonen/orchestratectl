---
created: 2026-06-28
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
labels: [review-spinoff]
closed: 2026-06-29
---

# Surface dropped-log count on error envelopes too

## Description

Follow-up from `log-delivery-hardening` (multi-model review finding, gpt-5.5 #4).

That issue surfaced the lossy-appender dropped-event count on the **success**
envelope (`output::emit_envelope` → `dropped_log_events` field + `warnings`
entry) and via the supervisor's periodic `warn!`. The **error** path does not:
a command that drops `error!`/`warn!` events and then fails emits a
`CliError` envelope (`crate::error::CliError::emit`, on stderr) with no
dropped-event count. That is arguably the worst case — the user loses logs
*and* gets an error, with no signal that logs were lost.

Scope: fold `crate::cli::dropped_log_events()` into the error envelope (and
its text-mode stderr rendering), mirroring the success-envelope treatment.
Decide whether the count belongs inside the `error` object or as a sibling
field. Keep it additive (no schema bump).

Out of scope in the parent because the parent's stated defaults limited the
envelope work to "commands that produce a success envelope".

