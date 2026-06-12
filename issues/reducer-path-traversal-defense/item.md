---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# Reducer needs path-traversal defense for IDs read from event log

## Description

The reducer reads IDs (`discussion_id`, `proposal_id`, `child_run_id`,
`child_node_id`) from `events.jsonl` and joins them onto disk paths
without validation. CLI write paths (`event create`, `discussion resolve`)
do validate via `require_safe_id`, but the reducer is the integrity layer
that protects against:

- a manually-edited or corrupted event log
- future writers that bypass the CLI validators
- replay from an untrusted/restored backup

A corrupt log line with `"discussion_id": "../etc/passwd"` could write
outside the run directory on next replay. Either:

- `RunPaths::discussion()` etc. sanitize the input before joining, OR
- the reducer calls a `validate_id` helper at every event-log boundary.

Discovered during: discussion-cli review (history/review-discussion-cli.md F15).
