---
created: 2026-07-25
updated: 2026-07-25
type: improvement
reporter: jari
status: in-progress
priority: normal
related: ['@merge-skips-teardown', '@agent-died-merge-no-teardown-interactive']
labels: [merge]
---

# Reducer should adopt a late explicit-merge report (durable merge-completed event) so the supervisor stays the sole teardown actor

_Source: octl-core reducer / supervise teardown_

## Description

Follow-up from the merge-skips-teardown fix (llm-review, 4-model panel).

## Context
merge-skips-teardown was fixed by having `run merge` reclaim the worktree+branch inline on the swallowed-report path (crates/octl-cli/src/run/merge.rs, cleanup::reclaim_merged_worktree_branch). That fix is correct and shipped, but it works AROUND a deeper design flaw the reviewers unanimously flagged.

## Root flaw
octl-core `reduce_node_report` (crates/octl-core/src/reducer.rs:692) drops ANY node.report against an already-terminal node as a dead event — last_report is left untouched. So a watchdog agent-died false positive (or any prior terminal) that lands before an explicit `run merge` causes the late `via: explicit-merge` marker to never reach the projection. any_node_merged_explicitly never sees it, and the SUPERVISOR (invariant #5's canonical teardown actor) can never warrant teardown. The current fix compensates in the CLI, which means teardown ownership is split.

## Proposed proper fix (reviewer consensus)
1. Introduce a durable `node.merge_completed` (or let `explicit-merge` override a watchdog terminal) event the reducer ACCEPTS even for terminal nodes, projecting cleanup-authorization + a structured merge RECEIPT (branch, merged_tip OID, source, source_tip). An explicit user merge carries strictly higher-fidelity ground truth than a watchdog timeout and should win.
2. Have append_and_apply_* return whether the reducer APPLIED vs no-op'd, so `run merge` no longer infers adoption by re-reading last_report.via (fragile string check).
3. With the projection correctly reflecting the merge, the supervisor becomes the sole teardown actor again (restores invariant #5 fully) and `run merge` can drop the inline reclaim.
4. Optionally gate force -D on the receipt (branch still at merged_tip, worktree clean) instead of trusting merge.sh exit 0 alone.

## Also worth folding in
- The watchdog agent-died FALSE POSITIVE on long-lived interactive runs is the upstream trigger (see @agent-died-merge-no-teardown-interactive defect #1) — fixing that removes most swallowed-report cases.
- merge.sh could emit a structured merge receipt (currently only exit code signals success).

## Not doing now
This is a core reducer + event-schema change (hot files: reducer.rs, events.rs). The shipped inline-reclaim fix resolves the user-visible leak safely; this issue tracks the architectural cleanup.
