---
created: 2026-08-15
updated: 2026-08-15
type: feature
status: open
priority: normal
labels: [deferred]
---

# pi.dev background jobs extension for orchestratectl waits

## Description

## Decision
Build this in **0.3**, not 0.2.0. Ship the 0.2 refactor/thin-supervisor release first; only then add the pi.dev background-wait integration.

## Shape
Create a separate open source pi.dev extension repo, likely `pi-background-jobs`, rather than vendoring pi-specific event-loop code into orchestratectl.

Boundary:
- `orchestratectl`: remains the run-state owner and exposes `run wait`, `landed`, and JSON contracts.
- `pi-background-jobs`: supervises long-running waits inside pi's event loop, returns immediately to the agent, and injects a follow-up when the process completes.
- First adapter: `/orx-wait <run-id>` runs `orchestratectl run wait <run-id> --output json --timeout 6h` and injects a completion message with status, landed flag, and summary.

## Rationale
In pi.dev, a normal bash/tool `orchestratectl run wait` blocks the agent turn. A shell-backgrounded `run wait &` can finish, but pi receives no internal event. Extension-owned waiters are the correct integration point because pi can track the child process and wake the agent with `sendUserMessage(..., { deliverAs: "followUp" })`.

## 0.3 acceptance sketch
- Own repo for the pi extension.
- Commands: `/orx-wait`, `/orx-wait-list`, `/orx-wait-cancel`.
- Session-visible job list, with reload-aware persisted entries via `pi.appendEntry` or equivalent.
- Documented shutdown behavior: kill or intentionally orphan child waiters.
- Timeout default, e.g. 6h, to avoid immortal waits.
- orchestratectl docs mention the extension as the recommended pi.dev non-blocking wait integration, but orchestratectl remains usable without it.
