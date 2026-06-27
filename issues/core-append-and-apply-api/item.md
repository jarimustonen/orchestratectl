---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-27
---

# octl-core: append_and_apply_event single mutation API

## Description

Spin-off from state-schema-crate review (gpt-5.5 #18, #19). The crate exposes append_event, append_event_with_seq, apply_event, and the projection write helpers separately, which invites callers to skip the reducer or write projections without holding the flock. Introduce append_and_apply_event(paths, kind, node_id, key, data) as the one canonical mutation entry point under the lock; make projection write helpers and append_event_with_seq pub(crate).

## Comments

### 2026-06-27T12:50:35Z · @claude

Landed. append_and_apply_event is the one canonical mutation entry: acquires the per-run flock, folds idempotency via find_prior_with_key, appends, runs the reducer, returns AppendResult{seq, idempotent_replay, prior}. Footguns tightened: bare append_event deleted; append_event_with_seq/write_event_line -> #[cfg(test)]; apply_event + write_manifest/write_discussion/write_spinoff + find_prior_with_key -> pub(crate). write_node + append_and_apply_unlocked kept pub as the sanctioned lock-held composition path (supervisor batch + transactional CLI verbs). All CLI mutation sites migrated. 3 unit tests (success/replay/terminal-noop) + 3 test files converted (2 reducer integration -> canonical API, flock V4 stress moved in-crate). Per issue-owner decision: kind stayed &str (no EventKind enum — it never existed; event create needs open kinds). Build+test+clippy+fmt clean. 4-model /llm-review run: fixed run/create parent-log key scan (orphan-on-retry), event/create empty-key guard, find_prior_with_key visibility. Deferred (already-tracked or new spin-offs): locked-run-witness-type, append-and-apply-transactional-validation, torn-write-truncate-tail, core-idempotency-api, supervisor-state-not-event-sourced (new).
