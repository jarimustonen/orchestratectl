---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: open
priority: normal
epic: lifecycle-architecture-review
---

## Problem

`supervise::cleanup::rollup_status(paths, children_all_terminal)` was made
log-authoritative for the run's OWN leaf nodes (issue
`rollup-status-log-authoritative`), but the `children_all_terminal` argument it
receives is computed by the caller (the supervisor tick in `supervise/mod.rs`)
and is still projection-derived for CHILD runs (a driver's tracked child runs).

The same crash window the leaf-node fix closed exists one level up: a child run
whose terminal `run.status` event was fsynced but whose manifest projection
write was crash-interrupted could be read as still-live (or a child whose
`child.spawned` was fsynced but manifest missing could be invisible), so a
driver could terminalize before a child actually settled — or hang.

Surfaced by the multi-model review of `rollup-status-log-authoritative`
(Gemini/Anthropic/DeepSeek all flagged it as a real gap but out of scope for the
leaf-node fix).

## Fix direction

Make the driver's child-terminality signal log-derived (per child run's event
log), symmetric with `read_node_statuses` for leaf nodes. Locate where
`children_all_terminal` is computed in `supervise/mod.rs` and base it on each
child run's log-authoritative status rather than its manifest/`nodes` projection.

## Notes

Hot-file cluster: `crates/octl-cli/src/supervise/*` — sequence edits.
