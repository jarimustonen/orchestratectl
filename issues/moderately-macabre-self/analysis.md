## Origin

Deferred from `run-paths-typed-selector-split`'s `/llm-review` (GPT-5.6-sol raised
it; all four reviewers agreed it is out of scope for the typed-selector split and
belongs in its own issue).

## Problem

`run_paths_exact(&RunId)` closes the **truncation → prefix** confused-deputy route:
a `<26`-char id can no longer fuzzy-resolve to some other run. It does NOT prove
the named run is actually a child of / related to the supervising parent. A corrupt
or forged persisted event carrying a **valid but unrelated** full ULID still
resolves to that other run, so a supervisor can be induced to:

- read another run's event log,
- signal termination based on another run's recorded pid,
- append `child.attached` / consume reports against an unrelated run.

`run_paths_exact`'s doc comment now explicitly notes this residual gap.

## Proposed direction

Before mutating or signaling a discovered child, validate the reciprocal
relationship from the child's own manifest/projection: the child's
`parent_run_id` must equal the supervising run and `parent_node_id` the expected
spawning node. On mismatch, surface `corrupt_state` (with both ids) rather than
proceeding. Relevant sites: `supervise/mod.rs` child-tail open, `signal_children_term`,
`record_child_attached`.

Design decisions to make: fail-closed vs. best-effort on a missing/racing child
manifest (child creation races supervisor tick); interaction with the existing
teardown gates.
