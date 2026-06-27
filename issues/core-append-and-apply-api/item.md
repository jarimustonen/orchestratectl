---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# octl-core: append_and_apply_event single mutation API

## Description

Spin-off from state-schema-crate review (gpt-5.5 #18, #19). The crate exposes append_event, append_event_with_seq, apply_event, and the projection write helpers separately, which invites callers to skip the reducer or write projections without holding the flock. Introduce append_and_apply_event(paths, kind, node_id, key, data) as the one canonical mutation entry point under the lock; make projection write helpers and append_event_with_seq pub(crate).
