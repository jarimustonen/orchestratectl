---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
epic: orchestratectl-mvp
---

# octl-core: store validated run_id in RunPaths

## Description

Spin-off from state-schema-crate review (gpt-5.5 #14). write_event_line derives run_id from RunPaths::root.file_name() which is brittle (symlinks, non-canonical paths) and silently produces empty strings. Refactor RunPaths::new to take run_id explicitly, validate it, store it, and use it for event envelopes. apply_event should also reject mismatched ev.run_id.
