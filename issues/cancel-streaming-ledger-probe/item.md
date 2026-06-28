---
created: 2026-06-28
updated: 2026-06-28
type: improvement
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# run cancel: stream an envelope-only ledger probe instead of materializing the full log

## Description

# run cancel: stream an envelope-only probe instead of materializing the full log

Spun off from cancel-pair /llm-review (gpt-5.5, opus-4.7, deepseek).

read_cancel_ledger calls read_all_events, which deserializes EVERY event —
including multi-KB node.report payloads — into a Vec<Event>, all under the held
run lock, before any write happens. For a long-running orchestration with
hundreds of nodes and rich reports this increases both lock-hold time (blocking
supervisors / report ingestion) and peak memory, a regression versus the old
nodes/*.json scan.

The cancel ledger only needs three envelope fields per line: kind, node_id, and
idempotency_key (plus, for the re-fold path, the full data of the few
run-cancel:* events). find_prior_with_key already demonstrates the pattern: a
ProbeFields skim that parses only the envelope and materializes the payload for
the one matching line.

Fix direction: add a streaming scanner over PhysicalLineReader that yields
envelope-only probes, materializing data only for events whose idempotency_key
is in this run's run-cancel:<run_id>: namespace. Care required: any new physical
-line reader MUST share the exact torn-tail / CorruptEventLog policy that
read_all_events, find_prior_with_key, and recover_last_seq are carefully kept
aligned on — hence its own change rather than an inline tweak.
