---
issue: node-cli-read
created: 2026-06-12
type: discuss-items
---

# Node CLI — open discussion items

`/llm-review` (Gemini-3.1, GPT-5.5, Opus-4.7, DeepSeek) on the
initial `node {list,show,report}` implementation surfaced several
findings that are correct but span beyond this issue's scope. The
high-confidence FIX items were applied in the follow-up commit; the
items below are captured for future issues.

The shared themes are: (a) the `event create` / `node report` write
paths copy too much code that should live in `taskfleet-core`, and (b)
the reducer is permissive in places where the CLI now defends. The
reducer-side hardening overlaps with `run-cli-read/handoff.md` D4–D5
and should land alongside that work.

---

## D1. `find_prior_event` / `find_prior_report` should be a shared helper in `taskfleet-core`

**Raised by:** Gemini (#7), GPT-5.5 (#16), Opus (#16), DeepSeek (#6)
**Severity:** maintenance — duplicate scanners drift independently

`crates/taskfleet-cli/src/event/create.rs::find_prior_event` and
`crates/taskfleet-cli/src/node/report.rs::find_prior_report` are
near-identical line-by-line scanners of `events.jsonl` looking for
a matching `idempotency_key`. They share the `ProbeFields` /
`FullEventForReplay` deserialise types and the same torn-line
tolerance.

**Recommendation:** spin off `idempotency-lookup-into-core`. Lift
the scanner into `taskfleet_core::events` taking the kind as a parameter
and returning a typed `PriorEvent { seq, node_id, data }`. Both CLI
sites then call one function and the torn-final-line vs.
middle-corrupt-line behavior (D2) is fixed in one place.

---

## D2. Idempotency scanner tolerates corruption *anywhere*, not just a torn final line

**Raised by:** Gemini (#4), GPT-5.5 (#17)
**Severity:** correctness — double-append risk if a middle line is corrupt

Both `find_prior_event` and the new `find_prior_report` `continue`
on any JSON parse error. The doc-comment says "mirror the
torn-final-line tolerance of `recover_last_seq`", but the
implementation skips malformed lines *anywhere* in the file. A
corrupt middle line containing a matching key would be silently
ignored and the CLI would double-append.

**Recommendation:** fix in tandem with D1. The shared helper should
either (a) track whether the malformed line is the file's last
line and tolerate only that, or (b) fall through to a hard
`CorruptEventLog` error and let the caller decide. Probably (b),
because by the time `node report` is called the upstream
`recover_last_seq` has already accepted the same file — a parse
failure here implies inter-line corruption.

---

## D3. Move §7.3 payload validation into `taskfleet-core`

**Raised by:** Opus (#38)
**Severity:** consistency — supervisor can't reuse the CLI's validator

`validate_report_payload` lives in `crates/taskfleet-cli/src/node/report.rs`
and returns `CliError` directly. When `supervisor-process` lands and
needs to validate child reports before consuming them (§7.3 step 3),
it cannot call the same function — it would have to copy it or pull
the CLI as a dependency.

**Recommendation:** move the validator and its sub-validators to
`taskfleet_core::report` returning a domain `ReportValidationError`, and
have the CLI map that to `CliError`. Same shape as the
proposed reducer-validation work in `run-cli-read/handoff.md` D5.

---

## D4. Reducer should require `success` XOR `cancelled` on `node.report`

**Raised by:** Gemini (#1)
**Severity:** correctness — bypasses around CLI validation persist invalid state

`apply_node_report` (in `crates/taskfleet-core/src/reducer.rs`) treats
missing `success` AND `cancelled` as "no status change" and silently
leaves the node in its prior status with `last_report` populated.
The CLI now rejects this shape, but the reducer is the canonical
gate — a future write path (or a `node.report` line replayed from
a corrupt log) would still produce a dangling-terminal-state node.

**Recommendation:** combine with `run-cli-read/handoff.md` D5
(terminal-state guard) into one `reducer-state-machine-hardening`
issue. Either: reject reports with neither `success` nor
`cancelled` as `CorruptEventLog`, or make the reducer's status
transition rule explicit (`is_terminal()` + only-allow-cancel-to-
overwrite-non-terminal).

---

## D5. Terminal-state guard on `node report`

**Raised by:** GPT-5.5 (#10), Opus (#11)
**Severity:** correctness — agent can flip an already-terminal node

A second `node report` against an already-`done`/`failed`/`cancelled`
node currently overwrites `last_report`, `updated_at`, and `status`.
The fix could live in the CLI (reject with `node_terminal`) or the
reducer (no-op once terminal; per `run-cli-read/handoff.md` D5).

**Recommendation:** reducer side, bundled into the D4 work. The CLI
check would have to run inside the flock to be race-safe, and
duplicating it on the supervisor's `event create` path would mean
two enforcement sites again. One canonical reducer rule is simpler.

---

## D6. `--dry-run` + `--idempotency-key` semantics

**Raised by:** Opus (#19)
**Severity:** UX — dry-run lies about would-be replay

`--dry-run` currently short-circuits before the idempotency lookup,
so `--dry-run --idempotency-key k1` reports `event_seq: null` even
when a real call would have replayed an existing `k1`'s seq.

**Options:**
1. Run the lookup inside dry-run too and report
   `{event_seq: <prior>, idempotent_replay: true}`.
2. Reject `--dry-run` + `--idempotency-key` as conflicting flags.

**Recommendation:** option 1, after D1's shared scanner lands so
the dry-run lookup can be cheap (the scanner becomes the
canonical lookup, no longer per-verb).

---

## D7. Validator should support `--from-stdin`

**Raised by:** Opus (#20)
**Severity:** UX — agents would prefer to pipe

`node report` (and `event create`) only accept `--from-file`. An
interactive agent skill typically generates the payload in-memory
and would prefer `cat report.json | taskfleet node report
... --from-stdin`. Minor.

**Recommendation:** add `--from-stdin` (mutually exclusive with
`--from-file`) in a follow-up `cli-from-stdin` issue. Make sure the
1 MiB cap and the TOCTOU defense (`read_capped` in `node/report.rs`)
carry over.

---

## D8. `node show` text mode hides operational fields

**Raised by:** Opus (#21)
**Severity:** UX — text consumers see less than JSON consumers

The text branch of `node show` doesn't print `worktree_path`,
`branch`, `tmux_window`, `agent_pid`, or `last_report.summary`.
JSON consumers see them. Add to text output, or document the
asymmetry. Minor.
