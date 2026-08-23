---
created: 2026-08-22
updated: 2026-08-22
type: task
status: untriaged
priority: normal
provenance: other
provenance_detail: Phase 1 implementation candidate from worker telemetry design
source_ref: orchestratectl:01m0ncfdymcb0y72241p4q8nsz/implementation-candidate:adapter
originating_run: 01m0ncfdymcb0y72241p4q8nsz
originating_run_kind: spinoff
---

# Build the external pi worker telemetry adapter package

## Description

Create the separately owned pi.dev telemetry package specified by `issues/worker-telemetry-protocol/design.md`; no adapter implementation belongs in orchestratectl.

Use only documented public pi ExtensionAPI lifecycle events and `pi.exec`. Implement the normative told-state precedence, bounded active-tool map/wire metadata, 30/90-second hybrid lease, one-send-per-two-seconds coalescing, immutable sequence retries, idempotent open/reopen/stop behavior, privacy filtering, bounded shutdown flush, and a non-interactive version/integrity probe. Package it as a pinned, reviewable npm/git release with peer dependencies and conformance/failure tests. Never inspect pi sessions/logs, private managers/EventBus, or background-process internals. This candidate remains untriaged until human protocol approval.
