---
created: 2026-06-12
updated: 2026-06-27
type: feature
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# Move idempotency event-log scanner to octl-core

## Description


`crates/octl-cli/src/event/create.rs::find_prior_event` and `crates/octl-cli/src/node/report.rs::find_prior_report` are near-identical line-by-line scanners of `events.jsonl` looking for a matching `idempotency_key`. They share the `ProbeFields` / `FullEventForReplay` deserialise types and the same torn-line tolerance.

Lift the scanner into `octl_core::events` taking the kind as a parameter and returning a typed `PriorEvent { seq, node_id, data }`. Both CLI sites then call one function.

While at it, fix the tolerance bug surfaced by review: both scanners `continue` on any JSON parse error (per `find_prior_event` in `event/create.rs:425` and `find_prior_report` in `node/report.rs`), but the doc-comment claims to mirror `recover_last_seq`'s torn-final-line tolerance. A corrupt middle line containing a matching key would be silently ignored and the CLI would double-append.

The shared helper should either (a) track whether the malformed line is the file's last line and tolerate only that, or (b) fall through to a hard `CorruptEventLog` error. Probably (b), because by the time the CLI is called the upstream `recover_last_seq` has already accepted the same file — a parse failure here implies inter-line corruption.

Sources: `issues/node-cli-read/handoff.md` D1, D2.
