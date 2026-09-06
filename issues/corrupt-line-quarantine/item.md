---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-28
commits:
- hash: 71b05e1
  summary: quarantine corrupt event-log lines
---

# Corrupt-line durability: quarantine/sidecar so strict replay survives a poisoned events.jsonl

## Description

From event-log-durability-trio review (GPT-5.5 #9, Opus #15). The supervisor now skips a corrupt middle line IN MEMORY and keeps tailing, but the bytes stay on disk: a later read_all_events / rebuild_projections still hard-errors on them, and the supervisor.event_log_skipped_line diagnostic is unreachable to strict replay (the corrupt line before it aborts the read). Decide and implement a durability policy: (a) quarantine — under the run lock, copy the corrupt physical line to events.corrupt/<offset> and rewrite events.jsonl without it + append a repair marker; or (b) sidecar — write skip diagnostics to a separate events.skipped.jsonl and keep events.jsonl canonical; or (c) an explicit tolerant-replay mode for diagnostics/TUI with strict replay remaining the default. After fixes #1+#3 a corrupt middle line should only arise from external tampering or bit rot, so this is a safety-net, not a hot path. Source: history/review-event-log-durability-trio.md (S2).

## Resolution — policy (a) quarantine (done under `events-tightening-pair`)

Implemented the quarantine policy; (b)/(c) rejected (a sidecar leaves the poison in the canonical log; a tolerant-replay mode weakens the strict-default the rest of the durability work depends on).

**Core** — `taskfleet_core::quarantine_corrupt_lines(paths, backup_ts)` (`crates/taskfleet-core/src/events.rs`): under the run `flock`, rename `events.jsonl` → `events.jsonl.corrupt-<ts>.bak` (verbatim original, kept for forensics), then atomically write a recovered `events.jsonl` holding every line *except* the corrupt ones. "Corrupt" = exactly what the strict readers reject (a newline-terminated, non-empty line that fails the full `Event` parse), so the recovered log is guaranteed to replay strictly. Empty lines and a torn (newline-less) final line are retained verbatim — the readers already tolerate both. Returns `Some { backup_path, removed_byte_offsets }` when it healed something, `None` when the log is missing/already clean (cheap: one read, no rename). An `_unlocked` sibling exists for lock-held composition. Symlink-guarded via `checked_events`, same as the append path.

**Supervisor** — quarantine is **on by default** (`--no-quarantine-corrupt-lines` opts out, restoring the P2 in-memory-skip behavior). When a tail parks at a corrupt line, `report_corrupt_line` (`crates/taskfleet-cli/src/supervise/mod.rs`) heals the owning run's log, restarts that tail at offset 0 (every byte offset shifted; `last_seq` preserved so already-consumed events are *not* reprocessed), and emits a single `supervisor.event_log_quarantined { backup_path, removed_byte_offsets, source }` on our own run log. Applies to both the own-run tail and child-run tails (child healed under the child's own lock). If quarantine fails, it falls back to the one-shot `supervisor.event_log_skipped_line` so the tail still progresses. This also fixes the P2 corner where the skip diagnostic couldn't be persisted when the corrupt line was the log's final record — after the heal the append lands on a clean file.

**Operator recovery:** the dropped bytes survive in the `.bak` named by the event. To restore a salvageable record: stop the supervisor, open the `.bak`, inspect the line(s) at `removed_byte_offsets`, hand-fix the JSON, append the corrected line to the live `events.jsonl` (or replace the file from a fixed copy of the backup), then restart the supervisor. The healed log is the source of truth; projections rebuild from it. (Also documented on the `quarantine_corrupt_lines` doc-comment.)

**Tests:** core unit tests (excise-middle-line + recover, clean-log no-op, missing-log None, torn-tail preserved) in `events.rs`; CLI integration `corrupt_tail_line_is_quarantined_and_log_heals` (+ the opt-out path retained as `corrupt_tail_line_is_skipped_once_without_looping`) in `tests/supervise_gates.rs`.
