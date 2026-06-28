---
created: 2026-06-27
updated: 2026-06-28
type: improvement
assignee: jari
status: wontfix
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
commits:
- hash: f0ec2b4
  summary: dedup-scan bench, defer index to F12
---

# find_prior_with_key: full-envelope strictness vs perf, and an idempotency index for O(n) scans

## Description

From event-log-durability-trio review (GPT-5.5 #1b/#10, DeepSeek #5). Two related points on the dedup scan: (1) find_prior_with_key parses a lenient subset (ProbeFields/FullEventForReplay) that does NOT require ts/run_id, so it can accept a newline-terminated line that read_all_events rejects as a non-Event — the 'all readers agree' framing is precise only about the torn-tail policy, not the full envelope schema (now documented). Decide whether to tighten the scan to a strict envelope parse (perf cost: validates every line on the hot dedup path) or keep it lenient-by-design. (2) Each append with an idempotency key does an O(n) full-log scan; on a long-lived run that is quadratic. Consider a compact on-disk idempotency index (kind+key -> seq) maintained under the lock. Source: history/review-event-log-durability-trio.md (S3 / Declined-for-now).

## Resolution — `wontfix` (deferred to F12)

Done under the `events-tightening-pair` wrapper. Per that task's MVP guidance, instead of building the index speculatively we benched the worst-case scan and let the number decide.

**Bench:** `crates/octl-core/benches/idempotency_scan.rs` (`cargo bench -p octl-core --bench idempotency_scan`). Each timed iteration is one full linear scan of an N-line log — the lookup key sits on the LAST line, so every line is parsed before the match. Triggered through the only public path (`append_and_apply_event` with an already-present `idempotency_key`, which returns the prior event before any append/reduce), so the measurement is lock-acquire + `find_prior_with_key` and nothing else. Apple Silicon, `--release`:

| N (log lines) | min | median | mean | p99 | max |
|---|---|---|---|---|---|
| 1,000 | 0.27 ms | 0.36 ms | 0.40 ms | 0.91 ms | 1.14 ms |
| 10,000 | 2.31 ms | 2.50 ms | 2.53 ms | 2.98 ms | 3.63 ms |
| 100,000 | 24.0 ms | 24.9 ms | 25.1 ms | 27.3 ms | 32.0 ms |

Cost is cleanly linear (~10× per 10× N): ~0.25µs per line, serde-parse-dominated.

**Decision gate (set by the task):** *"If p99 is <10ms at 10k entries, close as wontfix."* Measured p99 at 10k ≈ **3.0 ms < 10 ms** → **wontfix**. The scan only runs on appends that *carry* an idempotency key (not every append), and it crosses the 10ms line only around ~40k log lines — well beyond an MVP run's lifetime. Building + persisting a `BTreeMap<key, seq>` index now would add a second on-disk structure to keep consistent under the lock for no MVP-visible win.

**Pointer:** [[runwriter-batched-append-api]] (F12) restructures the append path so the dedup lookup is served from in-memory writer state, obviating the per-append scan entirely — the index belongs there, not as a bolt-on here. Revisit if a run is ever expected to exceed ~40k events with frequent keyed appends before F12 lands.

Point (1) — lenient-vs-strict envelope on the scan — is left **lenient-by-design** (already documented on `find_prior_with_key`): `seq` is not a match key, so a lenient probe must skim past an envelope that happens to lack it rather than abort the scan before a later real match. Tightening would only move work onto the hot path for no correctness gain (interior corruption is already a hard `CorruptEventLog`).
