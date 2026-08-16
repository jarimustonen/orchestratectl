---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: done
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: 63aa894
  summary: make supervisor rollup_status log-authoritative
- hash: 06880c1
  summary: surface log-read errors + honest cost docs (llm-review)
closed: 2026-08-15
---

# Make supervisor rollup_status log-authoritative (not a projection scan)

## Description

## Problem

`supervise::cleanup::rollup_status` enumerates a run's nodes via `list_nodes`
(`nodes/*.json` projection scan) to decide whether to terminalize the run. A node
whose `node.created`/terminal event was fsynced to `events.jsonl` but whose
projection write was crash-interrupted is invisible to that scan. The supervisor
can then observe only a subset of nodes, find them all terminal, and roll the run
up — while a node still lives in the log. A later `rebuild_projections` resurrects
that node under an already-terminal run, violating the core invariant "a run must
not terminalize while a log-visible node is live".

Surfaced by the `per-node-run` review (DeepSeek). It is **pre-existing** supervisor
behavior, not introduced by per-node cancel. The per-node-cancel terminalization
path is already log-safe: `cancel_node` self-rolls-up from the log-authoritative
`read_cancel_ledger` aggregate. This issue is only about the supervisor's own
per-tick rollup.

## Fix direction

Base the terminalization decision on the same streaming log replay `read_cancel_ledger`
uses (in `octl-core/src/cancel.rs`) rather than `list_nodes`. Options:
- expose a log-derived per-node-status reader from `octl-core` and have
  `rollup_status` call it, or
- move the roll-up classification into core entirely (it already owns
  `aggregate_terminal_status`).

The teardown loop can keep scanning projections for cleanup work; only the
run-status decision must be log-derived.

## Regression test

- bootstrap 2 nodes, delete `nodes/n-0002.json`, mark `n-0001` terminal
- assert `rollup_status(..., true)` is `None` (n-0002 still live in the log)

That test fails with the current projection-scan implementation.

## Comments

Hot-file caution: `crates/octl-cli/src/supervise/*` is a sequence-edits cluster.
Consider performance of a per-tick log scan vs the current directory read.
