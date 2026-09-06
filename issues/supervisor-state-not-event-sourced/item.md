---
created: 2026-06-27
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
related: ['@core-append-and-apply-api', '@append-and-apply-transactional-validation']
closed: 2026-06-29
---

# Supervisor projection-only state is not event-sourced (lost on rebuild)

## Description

Spin-off from core-append-and-apply-api multi-model review (gemini/gpt-5.5/opus, finding #4).

The supervisor writes two node-projection fields directly via the (deliberately still-`pub`) `taskfleet_core::write_node`, holding the run flock, with NO backing event:
- `Node.supervisor_pid` (supervise/mod.rs ~601, on child supervisor attach)
- `Node.last_processed_report_seq_by_child` (supervise/reducer.rs ~263-270, the report cursor mirror)

These fields are therefore NOT derivable from events.jsonl. A future `rebuild_projections_from_events` (replaying via the reducer) would silently zero them out, breaking disaster-recovery / projection-rebuild correctness.

Options:
1. Principled: emit events the reducer folds — e.g. `child.supervisor_attached` (already emitted to the PARENT log, but doesn't set the CHILD node's supervisor_pid) and a `child.report_consumed`/cursor-advance event. Gives free audit history too.
2. Move these to supervisor-private state files (SupervisorState already holds the cursor of record; the node-projection copy is a documented debugging mirror) and stop mirroring onto the node projection.

context: core-append-and-apply-api kept write_node pub as the sanctioned lock-held composition path per the issue owner's explicit decision; eliminating it / making it rebuild-safe is this follow-up. Distinct from append-and-apply-transactional-validation (reducer-error log poisoning) and manifest-counter-desync (counter repair).
