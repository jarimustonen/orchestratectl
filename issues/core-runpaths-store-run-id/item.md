---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-27
---

# octl-core: store validated run_id in RunPaths

## Description

Spin-off from state-schema-crate review (gpt-5.5 #14). write_event_line derives run_id from RunPaths::root.file_name() which is brittle (symlinks, non-canonical paths) and silently produces empty strings. Refactor RunPaths::new to take run_id explicitly, validate it, store it, and use it for event envelopes. apply_event should also reject mismatched ev.run_id.

## Comments

### 2026-06-27T07:07:20Z · @claude

Refactored RunPaths::new(root, run_id) to validate (lowercase 26-char Crockford ULID) and store run_id; replaced root.file_name() derivation in write_event_line/append_and_apply_unlocked with paths.run_id; added cross-id guard in apply_event (CorruptEventLog on ev.run_id != paths.run_id). Updated all call sites + doctor/snapshot test fixtures. Added unit tests for malformed-id rejection and the cross-run guard. Build/test/clippy/fmt clean.

### 2026-06-27T07:19:20Z · @claude

llm-review (gemini-3.1/gpt-5.5/opus-4.7/deepseek-v4) triage applied: (FIX) run_paths surfaces invalid_run_id with reason instead of collapsing to run_not_found; (FIX) doctor warns on invalid run-dir names instead of skipping, data check ungated so stale pids still caught; (FIX) tightened apply_event mismatch message; (FIX) added tests for envelope-stamps-paths.run_id, generator/validator lockstep, first-char boundary, CLI invalid_run_id vs run_not_found, doctor invalid-dir warn. (DEFER→F6 core-path-traversal-id-validation) enforce root.file_name()==run_id at construction + validate payload run_id refs via typed RunId newtype. (WONTFIX) legacy empty/non-canonical envelope migration — moot, new_run_id() only emits lowercase ULIDs, no released persisted state. Build/test/clippy/fmt clean. Report: history/review-core-runpaths-store-run-id.md

