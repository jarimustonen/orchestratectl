---
created: 2026-06-28
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-28
---

# run cancel: derive node/run liveness from the event log, not stale projections

## Description

# run cancel: derive node/run liveness from the event log, not stale projections

Spun off from cancel-pair /llm-review (gpt-5.5, opus-4.7).

cancel_run_unlocked now enumerates the node set from events.jsonl (the source of
truth), but it still decides each node's (and the run's) liveness by reading the
projection (read_node_opt / read_manifest), which can lag the log.

Failure mode: a non-cancel terminal event — e.g. a node.report {success:true}
or a run.status:done — is appended+fsynced but the process crashes before
apply_event folds it. The projection still reads non-terminal. A subsequent
cancel sees it as live, appends a cancel event over it, and folds the projection
to Cancelled. On a future rebuild_projections the log replays the success/done
FIRST (→ Done) and the later cancel is dropped by the terminal guard — so the
rebuilt projection (Done) disagrees with the live projection (Cancelled), and
the CancelOutcome reported the node/run as cancelled when the authoritative log
says it was already terminal.

This is pre-existing (the old nodes/*.json scan had the same projection-derived
liveness) and was explicitly left out of cancel-pair's scope. The convergence
fix landed there only re-folds already-logged CANCEL events; it does not guard a
fresh cancel against an already-logged non-cancel terminal.

Fix direction (needs its own design): compute authoritative node/run status by
replaying the reducer over the log under the lock, OR land a rebuild_projections
primitive and fold projections from the log before the cancel decision. The
parent issue (cancel-enumerate-from-event-log) gated exactly this on a
rebuild_projections primitive that does not yet exist.
