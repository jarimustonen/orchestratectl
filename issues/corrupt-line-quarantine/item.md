---
created: 2026-06-27
updated: 2026-06-27
type: improvement
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# Corrupt-line durability: quarantine/sidecar so strict replay survives a poisoned events.jsonl

## Description

From event-log-durability-trio review (GPT-5.5 #9, Opus #15). The supervisor now skips a corrupt middle line IN MEMORY and keeps tailing, but the bytes stay on disk: a later read_all_events / rebuild_projections still hard-errors on them, and the supervisor.event_log_skipped_line diagnostic is unreachable to strict replay (the corrupt line before it aborts the read). Decide and implement a durability policy: (a) quarantine — under the run lock, copy the corrupt physical line to events.corrupt/<offset> and rewrite events.jsonl without it + append a repair marker; or (b) sidecar — write skip diagnostics to a separate events.skipped.jsonl and keep events.jsonl canonical; or (c) an explicit tolerant-replay mode for diagnostics/TUI with strict replay remaining the default. After fixes #1+#3 a corrupt middle line should only arise from external tampering or bit rot, so this is a safety-net, not a hot path. Source: history/review-event-log-durability-trio.md (S2).
