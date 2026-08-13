---
created: 2026-08-12
updated: 2026-08-13
type: improvement
status: obsolete
priority: normal
related: ['@agent-skips-run-merge-idle-pending']
closed: 2026-08-13
closed_by: adr-decision-2
---

# idle-unmerged CPU clock: monotonic Instant for elapsed time

_Source: crates/octl-cli/src/supervise/mod.rs::cpu_activity_clock_

## Description

`cpu_activity_clock` computes its rate window from Unix timestamps (`now.timestamp()`), so a wall-clock step (NTP correction, manual clock change) perturbs `dt`. The reopen fix hardened the `dt <= 0` case (backward/zero-width window makes no determination) and preserves `last_active`, so a backward step is safe (never stamps active), but a FORWARD jump can momentarily inflate a window and let a trickle read as active, briefly refreshing the clock.

Robust fix: carry a monotonic `std::time::Instant` per node for elapsed-time math and derive Unix time only for interop with commit/file mtimes.

Low priority — the current wall-clock handling is defensive, not incorrect. Surfaced by /llm-review (OpenAI #5) of the reopen fix for `agent-skips-run-merge-idle-pending`.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

The three-clock idle-unmerged synthesizer is deleted by the thin model — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
