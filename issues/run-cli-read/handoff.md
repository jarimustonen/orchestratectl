---
issue: run-cli-read
created: 2026-06-12
type: discuss-items
---

# Run CLI — open discussion items

`/llm-review` (Gemini-3.1, GPT-5.5) on commits 731177b + fc69586 + the
fix-up commit returned several findings that are correct but span beyond
this issue's scope. Captured here for future issues to pick up.

---

## D1. `child.spawned` is written to the parent before the child run dir exists

**Raised by:** Gemini, GPT-5.5
**Severity:** correctness — log-only orphan reference

`crates/taskfleet-cli/src/run/create.rs` follows design.md §7.2 step 3 → 4
ordering: append `child.spawned` to the parent's events.jsonl first,
then create the child dir + write `run.created`. If step 4 fails (out
of inodes, permissions, kill -9 between steps), the parent's append-
only log permanently references a child run that never materialized.

Both reviewers proposed reversing the order (child-first, then parent
event), but that breaks the §7.2 contract — the parent supervisor's
tail-follow uses `child.spawned` as the spawn trigger; if it sees a
child dir but no event, it never picks it up.

**Options:**
1. Add a compensating `child.spawn_failed` event on the parent log when
   step 4 fails, plus reducer handling.
2. Use a two-phase commit: write a `.pending` sentinel in the child dir
   first, then parent event, then promote `.pending` → real manifest.
3. Accept the risk in MVP, document the recovery procedure for a stuck
   orphan reference (manual `event create` of a synthesized
   `node.report` to mark the spawning node failed).

**Recommendation:** revisit when the supervisor lands and the orphan-
recovery path is exercised end-to-end. For MVP the failure window is
seconds wide.

---

## D2. Parent-node existence is not validated on child-spawn

**Raised by:** GPT-5.5
**Severity:** correctness — silent link drop

Currently `run create --parent-run-id P --parent-node-id PN` checks
only `parent_paths.manifest().exists()`, not that PN exists. If PN is
absent the reducer's `apply_child_spawned` silently no-ops (it reads
the parent node, sees `None`, returns `Ok(())`), so the parent →
child link never forms.

I added the strict check, then had to back it out: in MVP the parent
supervisor (which creates `n-0001`) does not exist yet, so an end-
to-end CLI-only test would always trip it. The reducer's tolerance is
the right behavior given that the parent supervisor's first action on
boot is to create `n-0001` and then replay events.

**Recommendation:** restore the strict check once `supervisor-process`
lands and the parent agent reliably creates its `n-0001` before any
child can be spawned. Until then, leave the comment in `create.rs`
marking this as deferred.

---

## D3. Idempotency-key flow is not atomic across retries

**Raised by:** Gemini, GPT-5.5
**Severity:** correctness — concurrent retry can create two runs

`crates/taskfleet-cli/src/idempotency.rs` does lookup → create-run →
store-key. Two concurrent `run create --idempotency-key K` calls can
both miss the lookup and both create different runs; last writer wins
the key file. The success first-call may also crash after `run.created`
is durable but before `idempotency::store()` runs, in which case a
retry creates a duplicate run.

GPT-5.5 proposed a reservation flow: per-key flock, `Reserved →
Committed` state on the idempotency record, request fingerprint to
reject mismatched parameters. That is the right shape but adds enough
moving parts that it deserves its own issue.

**Recommendation:** spin off `idempotency-key-reservation` after the
MVP CLI is in use and we observe how often concurrent retries actually
hit. The single-user workstation case is essentially never; this lands
when the CLI starts being invoked from cron / scripts.

---

## D4. Reducer is not crash-safe across the line-append → projection-apply boundary

**Raised by:** Gemini, GPT-5.5
**Severity:** correctness — projection drift survives forever

`taskfleet_core::append_and_apply` writes the event line, fsyncs, then
calls `apply_event`. If the process dies between those two steps, the
projection (manifest, node, discussion, spinoff) is permanently behind
the event log. The next mutation writes `seq + 1` against stale
projections and the drift accumulates silently.

GPT-5.5 also flagged that `apply_node_created` writes the node file
before incrementing `manifest.node_count` — same crash-window applies
within `apply_event` itself.

**Recommendation:** this is an `taskfleet-core` concern, not a CLI one.
Open `reducer-watermark` to add a `last_applied_seq` field to manifest
and a startup "catch-up" pass that replays unapplied events. Separately,
consider switching to "projections are pure caches; rebuild on demand"
and deleting the stored counters entirely.

---

## D5. Reducer allows transitions OUT of terminal states

**Raised by:** GPT-5.5
**Severity:** correctness — `run cancel` can be undone by a late `node.report {success: true}`

`reducer.rs` `apply_run_status` / `apply_node_status` / `apply_node_report`
unconditionally write the new status. So a node cancelled by `run
cancel` and then reporting success via the agent (race window: agent
was already writing the report when cancel arrived) flips back to
`done`, breaking cancellation semantics.

**Recommendation:** `taskfleet-core` issue. Add `is_terminal()` guard and
make every status reducer a no-op when current status is terminal.
Document the rule on `Status` itself.

---

## D6. Append after crash-truncated tail can corrupt events.jsonl

**Raised by:** GPT-5.5
**Severity:** durability — replay parser fails on first read after corruption

`recover_last_seq` discards a torn final line for seq recovery but
does not truncate the file. `O_APPEND` then writes the next event
immediately after the partial bytes, producing one invalid JSONL line
that breaks every subsequent `read_all_events`.

**Recommendation:** `taskfleet-core` issue. `recover_last_seq` should
return `(last_seq, append_offset)` and `append_event_with_seq` should
`set_len(append_offset)` before writing.

---

## D7. `from_core` collapses every core error to `io_error`

**Raised by:** GPT-5.5
**Severity:** UX — machine callers can't distinguish disk failure from corrupt log

`run/mod.rs::from_core` maps every `taskfleet_core::Error` (Io, Json,
CorruptEventLog, UnsupportedSchemaVersion) to the same `code:
"io_error"`. Agents can't tell "retry the IO" from "the data on disk
is bad and retrying won't help".

**Recommendation:** small fix, but should be done across all noun
modules at once when `node`/`event`/`discussion`/`spinoff` land. Add a
proper `From<taskfleet_core::Error> for CliError` with per-variant codes.

---

## D8. `run list --kind/--status` accept arbitrary strings instead of typed enums

**Raised by:** GPT-5.5
**Severity:** consistency / UX — typo silently returns empty list

`run create --kind` uses `ValueEnum`; `run list --kind` accepts any
string. A typo like `--kind tehcnical-decision` returns `(no runs)`
rather than rejecting the typo.

**Recommendation:** convert to `ValueEnum` filters when adding the
corresponding `Status` typed arg. Minor diff; deferred to keep
this issue's scope tight.
