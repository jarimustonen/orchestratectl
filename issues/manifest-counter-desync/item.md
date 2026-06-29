---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Reducer manifest counters can permanently desync after partial-write crash

## Description

Reducer's incremental manifest counter mutations (`open_discussions`,
`pending_spinoffs`, `node_count`) can permanently desync if
`write_discussion`/`write_spinoff`/`write_node` succeeds but the follow-up
`write_manifest` fails or the process dies between the two writes.

The reducer's idempotency guards (`if status == Resolved { return Ok(()); }`)
then prevent a replay from re-incrementing/decrementing the counter, leaving
the manifest permanently inaccurate.

This was flagged across all four LLM reviewers as the highest-priority
architectural hazard during the discussion-cli review. Local fix candidates:

- Drop cached counters from `manifest.json` entirely; compute on read from
  the projection directories. Simplest and least error-prone.
- Add `last_applied_seq` per projection so partial-write recovery has a
  reconciliation hook. More flexible but doubles the consistency surface.
- Add a `repair` subcommand that walks the event log and rebuilds
  projections from scratch. Reactive rather than preventive.

Discovered during: discussion-cli review (history/review-discussion-cli.md F13).
