# state-schema-crate — handoff

## What landed

`crates/taskfleet-core/` now implements design.md §1 (state schema), §4 (per-run flock + atomic writes), and the §1.4 event-append + reducer protocol. See the merge commit message for the full scope.

V4 stress test (release, M-series Mac, 50 threads × 1000 iters):

- 50 000 distinct monotonic `seq`, no torn lines — correctness gate **PASSED**.
- Lock-acquire latency: p50 ≈ 181 ms, p99 ≈ 639 ms, max ≈ 2.45 s.
- This is **dramatically higher** than the <10 ms p99 expectation in `validation.md` V4. The driver is fsync-per-append on contention; recovery scans the tail of `events.jsonl` on every short-lived call. Correctness is intact, but per-event throughput at peak is ~250 ops/s/run, not the implicit thousands the V4 budget assumed. Jari to record the number in `validation.md` and decide whether to invoke the documented fallback ("recommend supervisors batch their writes") or build the proper `RunWriter` API now (see spin-offs).

## Spin-offs created

| Slug | Title | Trigger |
|---|---|---|
| `runwriter-batched-append-api` | `RunWriter` guard with cached `next_seq` + batched fsync | Latency above V4 budget; review §1, §3, §18, §20 |
| `core-path-traversal-id-validation` | Validate IDs / typed ID newtypes in `paths.rs` to prevent traversal | Review §13 |
| `core-runpaths-store-run-id` | Store validated `run_id` in `RunPaths`; stop deriving from `file_name()` | Review §14 |
| `core-append-and-apply-api` | Single `append_and_apply_event` mutation API + hide raw write helpers | Review §18, §19 |

## DISCUSS — items not addressed in this PR

### D1. Schema-version check on writes (§15)

Reviewers (gpt-5.5) flagged that `write_*` helpers accept arbitrary `schema_version` values and only the read side validates. Mild safety improvement; trivial change but coupled to the bigger §18/§19 refactor that hides write helpers behind a controlled API. Deferred to that work — checking it twice would be redundant once writes are `pub(crate)`.

### D2. `read_all_events` doesn't verify monotonic `seq` (§22)

Reviewers wanted a `read_all_events_checked` variant that asserts `seq == prev + 1` and `run_id` matches the path. Useful for `run reattach` replay but not load-bearing for the current crate boundary. Add when the supervisor reattach issue lands.

### D3. `node.report` cannot regress with at-least-once delivery (§12)

The reviewer correctly notes that an older replayed `node.report` could overwrite a newer one because the reducer keys nothing. The MVP design assumes events are applied strictly in `seq` order (the supervisor consumes its own log forward), and the deterministic-ID dedup in §1.4 covers the spinoff/discussion path. If the parent supervisor ever consumes child reports out of order, we will need a per-reporter version field. Track if/when supervisor work surfaces a real interleave.

### D4. Silent no-op when projection file is missing (§17)

Reducers for status/report/etc. silently return `Ok(())` when the target node/manifest doesn't exist. The MVP supervisor protocol guarantees `*.created` precedes `*.status`/`*.report`, and out-of-order replay against a fresh projection root is expected (you build from the start of the log). Adding a strict-vs-lenient `ReduceMode` enum was deferred — the strict path is what `run reattach` will want, but it can ride along with that issue.

### D5. Reducer is structurally a "live mutation against existing projections" API

A subtle architectural critique: `apply_event` mutates whichever projections exist on disk, which conflates "apply the next event during normal operation" with "rebuild from scratch." Both work with the current idempotency contract, but the names don't broadcast which the caller intends. Worth revisiting when the `RunWriter` / `append_and_apply` API lands — at that point the public surface can be: `apply_event_live(...)` (the next-in-sequence path) and `rebuild_projections_from_events(...)` (the recovery path), with the lower-level reducer functions internal.

## Notes

- All review fixes for items §1–§11 from the multi-LLM pass were applied in the second commit on this branch. The handoff above covers only the remaining items.
- Two-LLM review (gemini-3.1-pro-preview + gpt-5.5) was used in lieu of the full 4-model panel to keep the autonomous loop bounded; the findings overlapped substantially across both reviewers and were already saturating.
