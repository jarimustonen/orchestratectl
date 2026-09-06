# Worker telemetry adapter contract v1

This directory is the stable, harness-neutral contract for an external adapter
that reports a worker's last-told activity to taskfleet. The adapter itself
is owned and distributed separately. This repository contains no adapter
runtime or pi event handling.

Machine consumers should read [`contract.json`](contract.json) and run the
bounded cases in [`conformance.json`](conformance.json). Their top-level
`schema_version` versions the document/fixture shape. Additive descriptive
fields and cases for already-required v1 behavior do not bump it. The strict
request is different: adding or changing a request field or enum is breaking
and requires a `protocol_version` bump because v1 endpoints reject unknown
request fields. Success and error envelope consumers must tolerate additive
response fields and error codes.

## Public endpoint

For a recorded pi candidate declaring `telemetry = "worker-v1"`, the launcher
executes the candidate's recorded user-owned argv as an exact prefix without
reloading profile configuration, then forwards workmux's existing `-- <prompt>`
suffix unchanged. It exports the following identity into that process. Initial
creation uses absolute attempt `0`; every supervisor retry uses
the node's new absolute attempt. Candidates without that recorded pi declaration
receive none of these values (inherited ambient values are removed).

The adapter, not the endpoint, captures the exact worker identity once from:

- `TASKFLEET_RUN_ID`: full, non-empty run ID;
- `TASKFLEET_NODE_ID`: non-empty node ID; and
- `TASKFLEET_ATTEMPT`: canonical unsigned decimal absolute attempt number
  (`0` or `[1-9][0-9]*`, at most `4294967295`).

If any value is absent, empty, or invalid, the adapter sends nothing for that
session and records only a local diagnostic. It never guesses attempt `0` and
never takes identity from lifecycle event data.

The adapter submits the strict request on stdin to this public Taskfleet
command. The request protocol remains v1 and every supported distribution
channel exposes the same `taskfleet` executable.

```text
taskfleet node telemetry update --input-file - --output json
```

The endpoint reads identity only from the request body. Input is capped at an
inclusive 4 KiB before parsing. The typed DTO and conditional rules are in
`contract.json`; a representative request is:

```json
{
  "schema_version": 1,
  "protocol_version": 1,
  "run_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
  "node_id": "n-0001",
  "attempt": 0,
  "state": "tool_running",
  "active_tool_count": 1,
  "tool_name": "bash"
}
```

`active_tool_count` and `tool_name` are optional, but legal only with
`tool_running`. A name requires an explicitly present count of exactly one.
Transitioning to `agent_active`, `settled`, or `shutdown` therefore removes
both metadata fields rather than carrying old values forward. Unknown fields
are rejected.

On success the command exits 0 and writes the normal taskfleet JSON
envelope to stdout:

```json
{
  "schema_version": 1,
  "data": {
    "accepted": true,
    "run_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    "node_id": "n-0001",
    "attempt": 0,
    "received_at": "2026-08-23T12:00:00Z",
    "expires_at": "2026-08-23T12:01:30Z"
  },
  "warnings": []
}
```

On failure it exits nonzero and writes the normal versioned error envelope to
stderr. Error codes are diagnostic and additive; the adapter must treat an
unknown future code exactly like any known failure. It does not retry the same
invalid request indefinitely or derive activity from the error.

The timestamps are server-owned. `expires_at` is exactly 90 seconds after
`received_at`; the sample is stale at `now >= expires_at`. `state_since` is
server-maintained and intentionally not returned to the adapter. An endpoint or
spawn error neither clears nor reinterprets the prior sample. Without a later
accepted update, that sample simply ages and becomes stale. Telemetry never
changes run truth, status, reports, wait settlement, retry, merge, outcomes, or
cleanup.

## Activity reduction

The harness-neutral fixture vocabulary maps to public pi hooks in
`contract.json`. For pi, `session_open`, `turn_start`, `turn_settled`,
`tool_open`, `tool_close`, and `session_close` bind respectively to documented
`session_start`, `agent_start`, `agent_settled`, `tool_execution_start`,
`tool_execution_end`, and `session_shutdown` hooks.

The four states, in highest-to-lowest precedence, are:

1. `shutdown` after session shutdown;
2. `tool_running` while any paired tool execution remains open;
3. `agent_active` while an agent run is active; and
4. `settled` for a live session with neither an active agent run nor an open
   tool.

Shutdown is latched for the adapter instance; later notifications do not reopen
it. `settled` and `shutdown` are observations, not completion. Tool references
may be held only in bounded memory to pair start/end notifications; they never
cross the endpoint. Duplicate and unmatched notifications affect local
diagnostics only.

`tool_running` may carry a count from 1 through 32. For more than 32 active
tools, both metadata fields are omitted; `tool_running` alone remains valid. A
`tool_name` is sent only when exactly one tool is active and its public name
matches `^[A-Za-z0-9_.:-]{1,64}$`. An invalid name is omitted, not transformed
from event contents. Multiple tools never carry a name. The metadata describes
the derived snapshot at send start and is not a progress measure.

## Sending and shutdown

The desired snapshot consists of `state`, `active_tool_count`, and `tool_name`.
A change to any of them, including metadata-only changes while still
`tool_running`, becomes immediately send-eligible. Adapter timers use a
monotonic clock. Subprocess starts are at least two seconds apart, measured
start-to-start, except for the final shutdown update.

While one send is in flight, later activity coalesces to the newest snapshot;
no second send starts concurrently. Unchanged live state refreshes every 30
seconds, anchored to the previous subprocess start whether that send succeeds,
fails, or times out. Missed refresh periods coalesce into one send when the
floor and current in-flight send permit it. Failure itself never creates a new
activity state or an immediate retry loop.

`session_close` latches `shutdown` as the final desired state. A shutdown send
does not wait for the normal two-second floor, but it remains subject to
single-flight and the remaining shutdown budget. Waiting for an existing send
and attempting the final update together have a two-second budget. Expiry ends
the flush without blocking pi further; failure is diagnostic only.

## Conformance fixture execution

`conformance.json` contains two intentionally separate families:

- `endpoint_cases` submit strict requests to the real public command. Setup
  objects describe semantic preconditions such as `current_attempt`; each test
  harness establishes those through its own facilities. Setup is not adapter
  behavior or another public taskfleet API.
- `adapter_sequences` are executed by the owning external package against a
  fake sender and virtual monotonic clock. This repository checks their
  consistency and submits their reference payloads to the endpoint, but does
  not claim to execute pi hooks, scheduling, coalescing, or shutdown callbacks.

For adapter traces, `steps` are inputs, `sender_results` script the fake
subprocess completions, and `expected_sends` are the exhaustive observed starts
through `observation_window_ms`. Exact milliseconds are deterministic virtual
time, not a wall-clock tolerance requirement. The deterministic tie order,
refresh anchor, payload composition, and shutdown-return meaning are defined by
the top-level `trace_semantics` object.

## Privacy boundary

The complete payload allow-list is the request DTO above. In particular, the
adapter must not send tool arguments, commands, paths, tool results, message
text, prompts, output, errors, provider or model identity, pi session identity,
or tool-call IDs. It constructs a fresh allow-listed DTO rather than redacting
an event object.

## Ownership exclusions

This v1 contract deliberately has no adapter runtime, taskfleet internal
imports, pi extension-manager or EventBus access, background-job manager,
private process IDs, log/session-file reads, package provenance or integrity
probe, capability secret, open/reopen handshake, sequence or epoch, launch
attestation, permission model, or trusted package root. The external package
uses public pi hooks and the public command only.
