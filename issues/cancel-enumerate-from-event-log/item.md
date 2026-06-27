---
created: 2026-06-28
updated: 2026-06-28
type: improvement
status: open
priority: normal
epic: orchestratectl-mvp
---

# run cancel: enumerate live nodes from the event log, not the projection directory

## Description

Spun off from run-cancel-terminal-run-semantics /llm-review (gpt-5.5, opus #16).

core::cancel_run's live_node_ids scans nodes/*.json. But the codebase explicitly documents (events.rs) that the event log is the source of truth and projections can lag: a node.created event can be appended+fsynced while its projection write never happened (crash window). cancel_run would then NOT see that node, append run.status: cancelled, and leave the event log with a created-but-never-cancelled node — which a future rebuild_projections could resurrect as live under a Cancelled run.

Also (opus #16): if nodes/ is missing but manifest.node_count > 0, cancel currently treats it as 'no nodes' and cheerfully cancels, potentially stranding in-flight agents.

Fix direction: replay events.jsonl in memory under the lock to derive authoritative live-node set, or rebuild projections under the lock before cancelling, or add a 'projections clean through seq N' marker and refuse/repair when projections lag.

Out of scope for the parent issue (projection-directory enumeration matched the pre-existing CLI behavior; this is a deeper event-sourcing consistency concern shared by other read paths). Depends on whether a rebuild_projections primitive lands first.
