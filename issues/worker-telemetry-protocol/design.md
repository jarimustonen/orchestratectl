# Worker telemetry protocol — simplified Phase 1 design

**Status:** approved product direction; design only
**Scope:** a harness-neutral advisory sample in orchestratectl and a separately owned pi.dev adapter

## 1. Product decision

Worker telemetry answers two questions for the calling agent:

1. what activity did the worker last tell us about; and
2. how fresh is that report?

The answer is advisory. A caller may use the state and freshness as evidence when
judging what to inspect or do next. Orchestratectl itself does not convert
telemetry into run truth: telemetry cannot synthesize a report, mark work landed,
change run or node status, satisfy `run wait`, classify a terminal outcome,
authorize retry, or authorize teardown. `run merge` remains the only success
truth. Existing told exits, cancellation, the confirmed-dead grace, typed
outcomes, merge recovery, and cleanup guards remain unchanged.

Keep the protocol deliberately small. It does not diagnose progress, health,
idleness, or wedging. A current `tool_running` report can describe a stuck tool;
a stale report can come from a failed adapter while the worker continues. The
calling agent sees those facts and applies judgment.

Phase 1 supports autonomous operation only for pi launched with the adapter.
Claude has no adapter and remains explicit-interactive until one exists. An
autonomous fallback candidate must also provide the required telemetry; fallback
cannot silently weaken that condition.

## 2. Last-told activity

The adapter reports one state:

- `agent_active` — pi reported that an agent turn is active;
- `tool_running` — one or more tool executions are currently open;
- `settled` — pi reported that its automatic agent work has settled; or
- `shutdown` — pi reported session shutdown and the adapter attempted a final
  update.

`settled` and `shutdown` are observations, not completion. The worker may still
need to commit and run `orchestratectl run merge`.

For `tool_running`, the adapter may also report a bounded
`active_tool_count` (1–32) and a sanitized `tool_name` only when exactly one tool
is active. It never sends tool arguments, commands, paths, results, message text,
prompts, output, errors, model/provider identity, pi session identifiers, or
call IDs. Tool names must match `^[A-Za-z0-9_.:-]{1,64}$`.

The adapter uses the following simple precedence: shutdown, then any active tool,
then an active agent turn, then settled. It keeps tool-call IDs only in memory to
pair public pi start/end events. Duplicate or unmatched events affect only local
diagnostics.

## 3. Freshness

The adapter sends immediately when the state changes and refreshes the unchanged
state every 30 seconds while the pi session is alive. Orchestratectl stamps the
receive time. A report is `current` for 90 seconds and `stale` at
`now >= received_at + 90s`.

These values are freshness bounds, not health or failure thresholds. Expiry is a
computed read view; it causes no run-state event or transition. A forward clock
jump may make a report stale early. If the clock moves materially behind the
stored receive time, the view is `clock_unreliable`, not falsely current. Tests
use an injected clock.

`state_since` is server-maintained and changes only when the state enum changes.
It supports honest wording such as “last told: tool_running, state reported for
8m, refreshed 12s ago.” It is not a measured duration or progress clock.

## 4. Harness-neutral update contract

The launcher gives the worker its exact full run ID, node ID, and absolute
attempt number through environment variables. The adapter submits those values
to a public command:

```bash
orchestratectl node telemetry update \
  --run-id "$OCTL_RUN_ID" \
  --node-id "$OCTL_NODE_ID" \
  --attempt "$OCTL_ATTEMPT" \
  --state tool_running \
  --active-tool-count 1 \
  --tool-name bash \
  --output json
```

A strict JSON request may instead be read from `--input-file <PATH|->`. Payload
flags and `--input-file` are mutually exclusive. Requests use
`schema_version: 1` and `protocol_version: 1`; unknown fields, invalid enum/field
combinations, oversized input, unknown nodes, terminal nodes, and attempts that
do not equal the node's current attempt are rejected without mutation.

Example request:

```json
{
  "schema_version": 1,
  "protocol_version": 1,
  "run_id": "01...",
  "node_id": "n-0001",
  "attempt": 0,
  "state": "tool_running",
  "active_tool_count": 1,
  "tool_name": "bash"
}
```

Example response:

```json
{
  "schema_version": 1,
  "data": {
    "accepted": true,
    "run_id": "01...",
    "node_id": "n-0001",
    "attempt": 0,
    "received_at": "2026-08-23T12:00:00Z",
    "expires_at": "2026-08-23T12:01:30Z"
  },
  "warnings": []
}
```

There is no bearer capability, adapter registry, package-integrity proof,
open/epoch/incarnation handshake, client sequence, permission broker, or trusted
launch-composition protocol in v1. Agents have normal user rights, so such
machinery would not create a meaningful same-user security boundary. Attempt
matching prevents an old retry from being presented as the current attempt; an
older adapter incarnation within the same attempt may overwrite this advisory
sample, which is acceptable for v1 because the sample has no authority.

The endpoint validates under the ordinary run lock and atomically replaces one
bounded sample. It never writes projections directly from the adapter. Request
and stored sample size are capped at 4 KiB. The adapter coalesces event bursts,
keeps at most one subprocess send in flight, and sends at most once every two
seconds except for a best-effort shutdown update. Endpoint failure leaves the
previous sample to become stale; it never blocks the pi turn indefinitely.

## 5. Persistence and read surfaces

The replaceable sample lives at:

```text
runs/<run-id>/telemetry/<node-id>.json
```

It is advisory projection data, not an append to `events.jsonl`, and it does not
advance `manifest.applied_seq` or ordinary manifest/node progress timestamps.
The stored fields are limited to matching run/node/attempt identity, state,
sanitary tool metadata, `state_since`, `received_at`, and `expires_at`.
Corrupt or missing data reads as `invalid` or `absent`; it is never reconstructed
as authoritative state.

`run show --output json` exposes, per node:

```json
{
  "requirement": "required",
  "support": "configured",
  "sample": "current",
  "state": "tool_running",
  "age_ms": 12200,
  "state_elapsed_ms": 481000,
  "attempt": 0,
  "active_tool_count": 1,
  "tool_name": "bash"
}
```

- `requirement` is derived rather than stored: autonomous interaction means
  `required`, and explicit-interactive means `optional`; it describes the
  create-time selection condition only, with no run-state effect when absent;
- `support` is `configured | unsupported` from the selected candidate, not from
  whether samples happen to arrive; and
- `sample` is `absent | current | stale | clock_unreliable | invalid`.

On every read, a stored sample whose attempt differs from the node's current
attempt renders as `absent` regardless of its receive time. It is never shown as
activity for the current attempt.

Text uses observational language: “telemetry stale; last told activity:
`tool_running` 4m12s ago; run status unchanged.” It must not say “healthy,”
“making progress,” or “wedged.” `run list` may expose bounded counts by sample
state. `run wait` does not consume telemetry.

Calling agents are allowed to use these fields as evidence for their own next
step—for example, inspect a stale worker or wait longer after a fresh
`tool_running` report. That caller judgment does not mutate canonical run state
unless the caller invokes an existing explicit command whose normal rules allow
it.

## 6. Adapter and launch boundary

The adapter is a separate pi extension/package. It uses only documented public
pi lifecycle hooks (`session_start`, `agent_start`, `agent_settled`,
`tool_execution_start`, `tool_execution_end`, and `session_shutdown`) and a
bounded subprocess call to the public telemetry command. It does not import
orchestratectl internals, pi extension-manager internals, EventBus internals,
background-process managers, session JSONL, or private log files.

Executable agent commands and adapter arguments are user-owned configuration.
Repository configuration may select a named profile but cannot define or alter
commands or adapter paths. Orchestratectl does not auto-install, auto-update,
pin, attest, or police the adapter package and does not disable ambient pi
extensions. Installation and package trust remain ordinary user/operator
responsibilities, like other pi extensions with full user permissions.

A pi candidate is autonomous-eligible only when its user-owned definition says it
uses the v1 adapter. This is configuration eligibility, not attestation. If the
adapter or command is missing or fails at launch/runtime, existing simple agent
failure disclosure applies. Claude remains ineligible for autonomous selection
until a real adapter can produce this contract; explicit-interactive Claude may
run with `requirement=optional`, `support=unsupported`, and `sample=absent`.

Autonomous selection requires a candidate declaring `telemetry = "worker-v1"`.
The resolver's fallback rules, including residency preservation and retry
pinning, are defined in `../add-configurable-agent/design.md` §5 and are not
restated here.

## 7. State-integrity invariants

Telemetry must stay out of:

- `TerminalOutcome::classify` and `TerminalOutcome::teardown`;
- cleanup and work-preservation decisions;
- merge transactions, merge recovery, and landed checks;
- report synthesis and run/node status reducers;
- retry eligibility and the confirmed-death grace; and
- `run wait` settlement.

The removed commit-time, tmux-activity, CPU-rate, idle-unmerged, pane-streak, and
branch-reconciliation heuristics stay removed. Telemetry is a told advisory fact,
not a new spelling for those guesses.

Minimum negative tests cover stale/missing/corrupt samples, old attempts, long
tools, settled and shutdown reports, endpoint failure, event storms, malformed
payloads, clock jumps, retry/terminal races, and stripped ambient `PATH`. Every
case asserts that telemetry alone emits no report, status, retry, merge, or
cleanup effect.

## 8. Ownership and implementation boundary

Orchestratectl owns the strict update DTO, attempt validation, bounded atomic
sample, freshness rendering, read surfaces, and negative state-integrity tests.
The external adapter owns public-event translation, in-memory tool pairing,
coalescing/refresh, privacy filtering, and bounded shutdown behavior. End-to-end
work owns installation in an isolated test environment, autonomous pi and
interactive Claude validation, load checks, and rollout documentation.

This document does not implement production code or authorize scheduling. The
five already-filed untriaged implementation candidates remain for separate human
disposition; the post-decision assessment is recorded in
`../worker-control-plane-review/integration-review.md`.
