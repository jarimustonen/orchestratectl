---
created: 2026-08-15
updated: 2026-08-15
type: feature
status: open
priority: normal
---

# run salvage: fresh-agent continuation variant

## Description

Follow-up to `run-salvage-command` (thin-supervisor A3). The 0.2 `run salvage`
verb ships the **direct finish/merge** path: fence the prior worker, then drive
`run merge` from the preserved worktree's current git state (design.md §2.2
option (a)).

This issue tracks the deferred **fresh-agent continuation** (design.md §2.2
option (b)): instead of merging the worktree as-is, launch ONE fresh agent into
the SAME worktree to *continue* the work (never a second writer beside the
original — the original is fenced first, exactly as the merge path does). Likely
surface: `run salvage <id> --continue [--task <brief>]`, reusing the fence gate
and the single-node / preserved-worktree refusals already in `run/salvage.rs`.

Also in this bucket (from the original issue's option-1 review): per-node salvage
of a fan-out child (the current verb refuses multi-node with
`ambiguous_multi_node`), tracked with `per-node-run`.
