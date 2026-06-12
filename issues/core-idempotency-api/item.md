---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# Centralize --idempotency-key handling in octl-core::AppendOutcome

## Description

Both `event create` and `discussion resolve` now implement an
`--idempotency-key` event-log scan privately. Future domain verbs
(`spinoff approve|reject`, `run create`, `node report`) will need the
same semantics, and decentralizing it produced exactly the bug the
discussion-cli review caught (the first cut of `discussion resolve`
forgot to scan the log entirely).

Land a shared API in `octl-core::events`, e.g.:

```rust
pub enum AppendOutcome {
    Appended { seq: u64 },
    IdempotentReplay { seq: u64, prior_data: Value },
    Conflict { prior_seq: u64, prior_data: Value },
}

pub fn append_and_apply_idempotent<F>(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
    equivalent: F,
) -> Result<AppendOutcome>
where F: Fn(&Value, &Value) -> bool;
```

Then `event create` and `discussion resolve` use one call site; the
"equivalence" closure encodes the per-verb conflict matrix.

Discovered during: discussion-cli review (history/review-discussion-cli.md F14).
