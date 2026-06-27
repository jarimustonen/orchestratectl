---
created: 2026-06-27
updated: 2026-06-27
type: improvement
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# find_prior_with_key: full-envelope strictness vs perf, and an idempotency index for O(n) scans

## Description

From event-log-durability-trio review (GPT-5.5 #1b/#10, DeepSeek #5). Two related points on the dedup scan: (1) find_prior_with_key parses a lenient subset (ProbeFields/FullEventForReplay) that does NOT require ts/run_id, so it can accept a newline-terminated line that read_all_events rejects as a non-Event — the 'all readers agree' framing is precise only about the torn-tail policy, not the full envelope schema (now documented). Decide whether to tighten the scan to a strict envelope parse (perf cost: validates every line on the hot dedup path) or keep it lenient-by-design. (2) Each append with an idempotency key does an O(n) full-log scan; on a long-lived run that is quadratic. Consider a compact on-disk idempotency index (kind+key -> seq) maintained under the lock. Source: history/review-event-log-durability-trio.md (S3 / Declined-for-now).
