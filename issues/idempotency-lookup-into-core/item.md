---
created: 2026-06-12
updated: 2026-06-27
type: feature
status: done
priority: normal
epic: taskfleet-mvp
closed: 2026-06-27
commits:
- hash: b9d8dd2
  summary: add taskfleet_core::events::find_prior_with_key + PriorEvent
- hash: a3460bf
  summary: delegate event/create find_prior_event
- hash: b1d1664
  summary: delegate node/report find_prior_report
- hash: 48350df
  summary: torn-line semantics tests
- hash: 3b660fd
  summary: apply multi-model review findings
---

# Move idempotency event-log scanner to taskfleet-core

## Description


`crates/taskfleet-cli/src/event/create.rs::find_prior_event` and `crates/taskfleet-cli/src/node/report.rs::find_prior_report` are near-identical line-by-line scanners of `events.jsonl` looking for a matching `idempotency_key`. They share the `ProbeFields` / `FullEventForReplay` deserialise types and the same torn-line tolerance.

Lift the scanner into `taskfleet_core::events` taking the kind as a parameter and returning a typed `PriorEvent { seq, node_id, data }`. Both CLI sites then call one function.

While at it, fix the tolerance bug surfaced by review: both scanners `continue` on any JSON parse error (per `find_prior_event` in `event/create.rs:425` and `find_prior_report` in `node/report.rs`), but the doc-comment claims to mirror `recover_last_seq`'s torn-final-line tolerance. A corrupt middle line containing a matching key would be silently ignored and the CLI would double-append.

The shared helper should either (a) track whether the malformed line is the file's last line and tolerate only that, or (b) fall through to a hard `CorruptEventLog` error. Probably (b), because by the time the CLI is called the upstream `recover_last_seq` has already accepted the same file — a parse failure here implies inter-line corruption.

Sources: `issues/node-cli-read/handoff.md` D1, D2.

## Comments

### 2026-06-27T10:16:23Z · @jari

Multi-model review (Gemini 3.1, GPT-5.5, Opus 4.7, DeepSeek V4) applied: fixed a consensus-critical split-brain where a valid-JSON torn final line was matched by the dedup scan but discarded by recover_last_seq (lost-event / duplicate-seq). Also: ProbeFields.seq now optional, byte-oriented read (partial-UTF8 torn tail tolerated), matched-line bad payload + parse_seq now map to CorruptEventLog for consistent exit-1 classification, escape_debug excerpts. Report: history/review-idempotency-lookup-into-core.md. Two out-of-scope torn-line gaps (read_all_events divergence, append-path tail truncation) spun off to @torn-line-policy-consistency.
