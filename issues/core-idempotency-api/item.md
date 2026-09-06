---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Centralize --idempotency-key handling in taskfleet-core::AppendOutcome

## Description

Both `event create` and `discussion resolve` now implement an
`--idempotency-key` event-log scan privately. Future domain verbs
(`spinoff approve|reject`, `run create`, `node report`) will need the
same semantics, and decentralizing it produced exactly the bug the
discussion-cli review caught (the first cut of `discussion resolve`
forgot to scan the log entirely).

Land a shared API in `taskfleet-core::events`, e.g.:

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

## Decisions

### 2026-06-27T12:49:14Z · @claude

core-append-and-apply-api landed append_and_apply_event as a PARTIAL step toward this: it folds the find_prior_with_key scan + append into one flock window and returns AppendResult{seq, idempotent_replay, prior}. Conflict detection is still caller-side (event/create + node/report compare AppendResult.prior). Remaining: (a) move conflict/equivalence into core (the AppendOutcome enum + equivalence closure) so callers can't silently no-op a same-key/different-payload request; (b) reject empty keys IN CORE (CLI sites now do it at the boundary, but the core API still accepts Some("")); (c) review flagged the O(N)-per-keyed-append log scan under the exclusive lock — consider a sidecar idempotency index when this lands.
