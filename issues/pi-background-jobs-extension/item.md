---
created: 2026-08-15
updated: 2026-08-16
type: feature
status: obsolete
priority: normal
labels: [deferred]
closed: 2026-08-16
closed_by: claude
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

## Resolution

### 2026-08-16T15:33:24Z · @claude

Suljettu: kuuluu toiseen repoon. Issue päättää itse että tämä on erillinen avoimen lähdekoodin pi.dev-laajennus (`pi-background-jobs`) eikä osa orchestratectl:ää, ja ajoitus on 0.3. orchestratectl:n rooli (run wait, landed, JSON-sopimukset) on jo olemassa. Kirjaa uudelleen siinä repossa kun se perustetaan.

## Comments

### 2026-08-16T17:44:55Z · @claude

SUPERSEDED by homebase ADR 0011 (2026-08-16, Accepted): `0011-pidev-background-process-runtime.md`. Outcome differs from this issue's plan — no custom `pi-background-jobs` extension is being built. homebase conditionally adopts the pinned third-party `@aliou/pi-processes@0.10.9` as its INTERACTIVE, session-scoped pi runtime (gated on a smoke matrix); building our own extension was evaluated and rejected. The DURABLE, harness-neutral runner is separate work (`orx-background-runner`, homebase, blocked on that gate). Binding constraint recorded in this repo's AGENTS.md: orchestratectl must not import pi-processes, touch its manager, assume its ids/log paths, or use its in-process EventBus — it stays the run-state owner behind `run wait` / `landed` / the JSON contracts. Do not re-file a custom-extension issue here.
