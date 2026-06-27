---
created: 2026-06-27
updated: 2026-06-27
type: improvement
status: open
priority: normal
epic: orchestratectl-mvp
related: ['@id-canonical-form-validation']
---

# octl-core: append API should take Option<&NodeId> instead of Option<&str>

_Source: crates/octl-core/src/events.rs_

## Description

Spin-off from id-canonical-form-validation /llm-review (gpt-5.5, opus-4.7).

After the envelope-typing pass, `Event.run_id`/`node_id` are `RunId`/`Option<NodeId>`, but the core append API (`append_and_apply_event` / `append_and_apply_unlocked`, and the test-only `append_event_with_seq`) still takes `node_id: Option<&str>` and parses it late via `parse_envelope_node_id`, rejecting an invalid id with `Error::InvalidNodeId` at write time.

This is a half-measure: an invalid node id stays representable through most of the call graph, and the write path can fail with a bad-input error the core layer is the wrong place to raise. The cleaner shape is to accept `Option<&NodeId>`, push validation to the entry boundaries (CLI arg parsing — which already does `parse_node_id(..)?` then `.to_string()`s the result back — and serde deserialize), then `clone()` the typed id into the envelope and delete `parse_envelope_node_id` + `Error::InvalidNodeId`.

Scope: change the three append signatures to `Option<&NodeId>`; update the ~15 call sites (run/create, run/cancel, run/reattach, node/report, event/create, supervise/*, spinoff/*, discussion/resolve, core tests) — several already hold a `NodeId` and stringify it, so they get simpler; drop `parse_envelope_node_id`/`Error::InvalidNodeId` (or, if a raw-string entry point remains, give the error the structured `kind`/`expected` fields of `IdValidationError` instead of a flat `reason: String`).

Out of scope: the id charset/validation logic itself (already landed). Pure type-safety / API-shape hardening; no behavior change for valid inputs.
