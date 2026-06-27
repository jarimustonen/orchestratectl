---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# octl-core: store validated run_id in RunPaths

## Description

Spin-off from state-schema-crate review (gpt-5.5 #14). write_event_line derives run_id from RunPaths::root.file_name() which is brittle (symlinks, non-canonical paths) and silently produces empty strings. Refactor RunPaths::new to take run_id explicitly, validate it, store it, and use it for event envelopes. apply_event should also reject mismatched ev.run_id.

## Comments

### 2026-06-27T07:07:20Z · @claude

Refactored RunPaths::new(root, run_id) to validate (lowercase 26-char Crockford ULID) and store run_id; replaced root.file_name() derivation in write_event_line/append_and_apply_unlocked with paths.run_id; added cross-id guard in apply_event (CorruptEventLog on ev.run_id != paths.run_id). Updated all call sites + doctor/snapshot test fixtures. Added unit tests for malformed-id rejection and the cross-run guard. Build/test/clippy/fmt clean.
