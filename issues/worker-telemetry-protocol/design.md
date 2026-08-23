# Worker telemetry protocol — Phase 1 design

**Status:** proposed for human review; no production protocol or adapter is implemented here  
**Scope:** harness-neutral protocol in orchestratectl plus a separately owned pi.dev adapter  
**Validated against:** pi-coding-agent 0.84.2 installed documentation and examples

## 1. Decision

Use a **hybrid, replaceable lease**. An adapter tells orchestratectl when its
state changes and refreshes the current state every 30 seconds while the pi
session is alive. Orchestratectl stores one bounded, atomically replaced sample
per node. A lease is current for 90 seconds.

This protocol reports the last told lifecycle state and whether an authorized
writer refreshed it recently. It **cannot distinguish healthy progress from a
wedged model or tool operation**. A blocked tool can remain `tool_running` and a
faulty timer can remain current. Actual wedge detection is out of scope; adding
duration thresholds or progress inference would recreate the removed stall
heuristics.

Telemetry is diagnostic evidence only. It cannot change `Node.status` or
`Manifest.status`, synthesize a `node.report`, classify a `TerminalOutcome`,
trigger retry, satisfy `run wait`, authorize cleanup, or prove that work landed.
An absent, rejected, or expired lease means only **telemetry unavailable** and
may request attention. `run merge` remains the only success truth. A told
`worker.exited` failure and the existing fixed-grace confirmed-dead backstop
remain the terminal crash paths.

The first adapter is a separate, pinned pi package. Orchestratectl communicates
with it only through process environment plus a public CLI/JSON command. It does
not import pi packages, inspect pi's extension manager or EventBus, read pi
sessions/logs, or adopt pi process identifiers.

## 2. Why hybrid

| Approach | Benefit | Failure | Decision |
|---|---|---|---|
| Periodic heartbeat only | Detects loss and covers long model/tool work | A heartbeat says only that the timer ran; without explicit state transitions it has little diagnostic value | Reject alone |
| Event-only transitions | Low write rate and semantically precise | A long healthy tool looks identical to a dead adapter after the last start event; a lost end event remains current indefinitely | Reject alone |
| Hybrid lease | Explicit state plus bounded evidence that the adapter still runs; transitions are immediate and long work remains current | Requires expiry, coalescing, and clock rules | Adopt |

The 30/90-second ratio tolerates two missed refreshes. Both values are returned
by `open`, not guessed by the adapter. They are not stall, health, or failure
thresholds. Expiry changes only the telemetry sample view.

## 3. Facts and vocabulary

The adapter reports exactly one state:

- `agent_active` — pi emitted `agent_start`, or a tool ended while the agent run
  remains unsettled;
- `tool_running` — at least one `tool_execution_start` has not yet had its
  matching `tool_execution_end`;
- `settled` — pi emitted `agent_settled`; pi has no automatic retry,
  compaction retry, or queued continuation left;
- `shutdown` — pi emitted `session_shutdown` and the adapter attempted its final
  update.

`shutdown` and `settled` are not completion. A settled worker may still need to
run the closing workflow; shutdown may be reload, session replacement, or quit.
Neither is terminal authority.

The wire vocabulary deliberately omits `success`, `failed`, `wedged`, `idle`,
`thinking`, progress percentages, and inferred activity. `agent_active` means
only that the documented lifecycle says an agent run is in progress. A current
lease does not prove useful progress; a wedged event loop may stop refreshing
and become stale, while a faulty timer could remain current in a wedged agent.
The UI must say what was observed, not overclaim diagnosis. In particular,
`agent_active` must not be rendered as “thinking” or “making progress”,
`tool_running` does not prove the tool process is healthy, and `current` does not
mean productive.

Normative state precedence is: observed shutdown → `shutdown`; otherwise a
non-empty active-tool map → `tool_running`; otherwise an agent generation that
has started but not settled → `agent_active`; otherwise an observed settlement
→ `settled`; otherwise no sample. A later `agent_start` starts a new unsettled
generation. Duplicate starts replace the map entry; unmatched/duplicate ends are
ignored and counted only in adapter-local diagnostics. Shutdown clears the map.
An impossible `agent_settled` while tools remain active stays `tool_running`
until those tools end, then becomes `settled`.

Wire validity is strict:

| State | `active_tool_count` | `tool_name` | `count_truncated` |
|---|---|---|---|
| `agent_active` | absent | absent | absent |
| `tool_running` | required, 1–32 | optional only when count is 1 and not truncated | present and `true` only at saturation |
| `settled` | absent | absent | absent |
| `shutdown` | absent | absent | absent |

### Parallel tools

The adapter tracks `toolCallId` only in memory to pair public start/end events.
It reports `active_tool_count` (integer, 1–32, saturated with an explicit
`count_truncated: true`) and reports `tool_name` only when exactly one tool is
active. It never transmits call IDs. When several tools run, the display says
`3 tools running`, not an arbitrary tool name. Completion-order end events and
interleaved update events are supported.

### Privacy

Permitted fields are state, bounded count, and a tool registration name matching
`^[A-Za-z0-9_.:-]{1,64}$`. Tool arguments, commands, paths, results, partial
results, message text, model/provider names, session IDs/files, prompts, output,
and errors are forbidden. Orchestratectl validates and rejects unknown or
oversized fields. The adapter does not log payloads by default. `--verbose`
diagnostics contain identity and state but never the capability secret or tool
arguments.

Elapsed time is computed by the read surface from orchestratectl's accepted
`state_since`, not supplied by the adapter. Only a change of the state enum
resets `state_since`; count/name changes and refreshes do not. Orchestratectl
computes that comparison from canonical accepted bodies. The value means time
since that lifecycle enum was accepted, not actual operation duration.

## 4. Identity, attempts, and authorization

### Capability file

Before launching a telemetry-required worker attempt, orchestratectl creates a
capability file under its own run home, outside the worktree:

```text
~/.orchestratectl/runs/<run-id>/secrets/<node-id>-attempt-<n>.json
```

The run directory and `secrets/` are owner-only; the file is mode 0600 and is
created without following symlinks. It contains protocol version, exact full run
ID, node ID, absolute attempt number, expiry policy, a random 256-bit bearer
secret, and an adapter audience. The node projection stores only a hash and
capability generation, never the bearer secret. The launcher passes only the
file path in `OCTL_TELEMETRY_CAPABILITY`; the secret is not placed in argv,
prompt text, git state, or general configuration.

This authorizes writes for the current attempt and protects against accidental
cross-run writes, stale copied files, edited identity metadata lacking the
current random value, and other local users. It does not authenticate adapter
provenance or sandbox arbitrary code already executing as the same OS user:
such code can read user-owned process state and files. Pi extensions have full
user permissions, so package trust is the primary same-user boundary.

On every `node.retry`, orchestratectl revokes/removes the old capability and
issues a fresh secret with the new absolute `retry_attempts` value. The endpoint
compares run, node, attempt, generation, and secret hash under the run lock.
Old-attempt telemetry is rejected before any write. Terminalization records
revocation in authoritative state and removes the capability. A missing
capability after a previously successful open is a permanent local stop
condition; the design does not promise `telemetry_closed` when identity can no
longer be read safely from the deleted file.

### Adapter incarnation fence

Pi reload/new/resume/fork tears down and rebinds extension instances within the
same worker attempt. At each `session_start`, the adapter performs one bounded,
awaited `open` call with a random `client_instance_id`. A first open installs a
new epoch. Repeating `open` with the same current capability generation and
client ID is idempotent and returns the existing epoch. A different ID is an
explicit takeover: it installs a new epoch and atomically clears current-sample
eligibility, so readers show `awaiting_first_sample` rather than the prior
incarnation's state. Updates carry the epoch and a strictly increasing
`client_seq`.

Pi lifecycle ordering does not order subprocess completion. The last serialized
successful open for different IDs wins. If a live incarnation receives
`stale_telemetry`, it may reopen once with a new client ID; a second fence loss
permanently disables that incarnation. A stale/closed/revoked writer clears its
timer and queue instead of retrying forever. Same-ID retries handle a committed
open whose response was lost. The ID is opaque, bounded, and not a pi session
ID.

The **authoritative telemetry control** is separate from the disposable sample.
Rare capability/open/revoke events project attempt, generation, secret hash,
revocation, current client ID, and epoch into the node under the ordinary run
lock and `applied_seq` rules. The bounded telemetry sample contains only the
matching identity, client sequence, state, sanitized metadata, and timestamps.
Missing/corrupt samples degrade to unavailable without resetting the control
fence. Updates are refused on a corrupt sample until an idempotent/new `open`
clears it. Missing/corrupt authoritative control fails closed and requires
operator repair; it is never reconstructed from advisory data.

For one epoch:

- higher sequence: accept;
- same sequence and identical canonical body: idempotently return the existing
  acknowledgement without a write;
- same sequence with different body: reject `telemetry_sequence_conflict`;
- lower sequence or older epoch: reject `stale_telemetry`.

The sender snapshots one immutable body when allocating a sequence and retries
that exact pair until acknowledged or permanently rejected. Coalesced changes
receive a new sequence only after the in-flight pair resolves. Gaps are allowed.
A sequence conflict is an adapter defect: record a diagnostic and reopen at most
once rather than mutating the same sequence. Epoch and sequence are unsigned
32-bit integers; exhaustion requires a new open, and epoch exhaustion disables
telemetry for the attempt rather than wrapping.

## 5. Public CLI/JSON contract

The proposed resource is nested under `node` and uses canonical `update` rather
than a pi-specific verb:

```bash
orchestratectl node telemetry open \
  --capability-file "$OCTL_TELEMETRY_CAPABILITY" \
  --client-instance-id 01J... \
  --output json

orchestratectl node telemetry update \
  --capability-file "$OCTL_TELEMETRY_CAPABILITY" \
  --lease-epoch 4 --client-seq 18 \
  --state tool_running --active-tool-count 1 --tool-name bash \
  --output json
```

The implementation also accepts a strict JSON request from
`--input-file <PATH|->`. `--capability-file` remains required transport metadata;
payload flags and `--input-file` are mutually exclusive. Every request includes
`schema_version: 1` and `protocol_version: 1`. Unknown keys are rejected within
the negotiated protocol version; additive fields require a later negotiated
version. Unknown keys, invalid enums, inconsistent state fields, oversized
values, malformed capability files, and mixed identities fail clearly. There
are no prompts and no network service.

Canonical open request/response (flags normalize to the same DTO):

```json
{"schema_version":1,"protocol_version":1,
 "client_instance_id":"01J9H8X6N5M4K3J2H1G0F9E8D7"}
```

```json
{
  "schema_version": 1,
  "data": {
    "protocol_version": 1,
    "run_id": "01...", "node_id": "n-0001", "attempt": 0,
    "client_instance_id": "01J9H8X6N5M4K3J2H1G0F9E8D7",
    "lease_epoch": 4, "initial_client_seq": 0,
    "opened_at": "2026-08-22T12:00:00Z",
    "refresh_interval_ms": 30000, "lease_ttl_ms": 90000,
    "open_deadline_ms": 2000, "shutdown_flush_deadline_ms": 2000
  },
  "warnings": []
}
```

Canonical update request:

```json
{"schema_version":1,"protocol_version":1,"lease_epoch":4,"client_seq":18,
 "state":"tool_running","active_tool_count":1,"tool_name":"bash"}
```

Successful update response:

```json
{
  "schema_version": 1,
  "data": {
    "accepted": true,
    "idempotent": false,
    "run_id": "01...",
    "node_id": "n-0001",
    "attempt": 0,
    "lease_epoch": 4,
    "client_seq": 18,
    "received_at": "2026-08-22T12:00:00Z",
    "expires_at": "2026-08-22T12:01:30Z"
  },
  "warnings": []
}
```

Errors use the repository's standard versioned stderr envelope. Caller/domain
errors exit 1; I/O, lock, invariant, and malformed endpoint-response failures
exit 2. Stable protocol codes and sender actions are:

| Code | Mutation | Sender action |
|---|---|---|
| `invalid_telemetry`, `incompatible_telemetry_protocol`, `capability_invalid` | none | disable permanently |
| `telemetry_closed`, missing capability after open | none | disable permanently |
| `stale_telemetry` | none | reopen once, then disable |
| `telemetry_sequence_conflict` | none | diagnose and reopen once, then disable |
| `telemetry_lock_timeout`, `io_error`, endpoint exit 2/malformed response | none | bounded exponential backoff with jitter |

The adapter coalesces pending state to the latest snapshot but never changes an
in-flight sequence/body pair. It does not await ordinary sends in event handlers.
`session_start` open and `session_shutdown` final flush are the only awaited
calls, each with the returned 2-second deadline; failure allows pi to
continue/exit and merely leaves telemetry absent or stale.

Normative protocol-v1 limits are: request/response 4 KiB before JSON parsing;
ASCII tool name 64 bytes; client ID 26 ULID characters; epoch/sequence `u32`;
projection sample 4 KiB; refresh 30,000 ms; TTL 90,000 ms; maximum one accepted
update each 2,000 ms except the best-effort shutdown update; exact expiry at
`now >= expires_at`. RFC 3339 timestamps are UTC with millisecond precision.

## 6. Persistence and write bound

The current sample is a separate advisory projection:

```text
runs/<run-id>/telemetry/<node-id>.json
```

It is replaced atomically through the public endpoint and is deliberately not an
append to `events.jsonl`; a keepalive does not advance `manifest.applied_seq` or
manifest/node `updated_at`. The authoritative control described in §4 does use
rare normal events, so retry, terminalization, and open fencing remain inside the
existing `LockedRun`/reducer invariants.

The implementation must choose a lock/write ordering that checks authoritative
control and replaces the sample consistently. The conservative baseline is a
short existing-run-lock critical section; a separate telemetry lock is allowed
only with a proved ordering against retry/revoke/terminalization. This decision
is gated by contention tests: telemetry cannot hold the run lock during process
spawn, parsing outside the run, sleeps, retries, or any network/tool operation,
and endpoint lock acquisition is bounded. Direct adapter writes are forbidden.

The 4 KiB sample contains matching identity/epoch/sequence, state,
`state_since`, server `received_at`/`expires_at`, and sanitized tool metadata.
There is no transition ring or event-rate field; coalescing makes such history
lossy and invites activity inference.

The adapter sends at most one update per two seconds (latest state wins), with a
single immutable request in flight. Unchanged refreshes are therefore at most
120/hour/node and total accepted ordinary updates are at most 1,800/hour/node;
the final shutdown attempt is the sole extra. Backoff cannot spawn overlapping
children. The production slice must benchmark subprocess cost, lock wait, fsync
policy, aligned multi-node load, and long run-lock holders before rollout. A UDS
would require a resident-service architecture and direct projection writes would
violate ownership, so neither replaces the required CLI endpoint in Phase 1.

## 7. Clock and expiry semantics

The server stamps receive time; adapter clocks are ignored. Because every CLI
invocation is short-lived, expiry uses persisted server wall time; no in-memory
monotonic deadline is claimed across processes. Persisted read surfaces compute:

- `now >= received_at`: ordinary age calculation, stale exactly at
  `now >= expires_at`;
- a forward clock jump can make telemetry stale early, which is safe because
  stale is advisory only;
- `now` earlier than `received_at` beyond a small tolerance yields
  `sample: "clock_unreliable"`, never a falsely current lease;
- parse errors or impossible timestamps yield `sample: "invalid"` and a
  warning, not a terminal outcome.

Expiry is a computed view, not a timer-driven state mutation. Nothing is
appended when a lease crosses 90 seconds. Clock logic must use an injected clock
in tests. Supervisor restart does not extend a lease; if age cannot be trusted, it fails
closed to unavailable. A small negative age is clamped to zero; beyond a defined
implementation tolerance it is `clock_unreliable`. Clock correction may change
`stale` back to `current`; this is accepted advisory behavior and never authority.

## 8. Read surfaces

`run show --output json` exposes telemetry per node, assembled at the
presentation boundary after terminal/attention/stall calculations:

```json
{
  "requirement": "required",
  "support": "available",
  "sample": "current",
  "state": "tool_running",
  "age_ms": 12200,
  "state_elapsed_ms": 481000,
  "attempt": 0,
  "active_tool_count": 1,
  "tool_name": "bash"
}
```

Dimensions are separate:

- `requirement`: `required | optional` from launch policy;
- `support`: `available | unsupported | incompatible` from the resolved harness
  capability/probe, never inferred from samples;
- `sample`: `absent | awaiting_first_sample | current | stale |
  clock_unreliable | invalid | closed`.

`awaiting_first_sample` means authoritative control has accepted the current
incarnation but no matching update. A sample from a prior attempt/generation/
epoch is historical and never eligible for `current`. `closed` means the node is
terminal; retained last-sample health may be shown separately as
`historical_sample`. Text says `telemetry stale (last told observation:
tool_running 4m12s ago); run status unchanged`, never “worker wedged”.

`run list` does not choose one node or invent a worst-state aggregate. It exposes
counts by `sample` value plus `telemetry_required_nodes` and
`telemetry_supported_nodes`. Detailed state remains in `run show` node rows.
`run wait` v1 does not include telemetry and does **not** settle because telemetry
is stale, shutdown, or settled. Any addition to waiting is a separate
human-reviewed decision.

No generic attention, health, progress, last-activity, sorting, retry, stint, or
cleanup consumer may derive automation from telemetry or its durations. This
prohibition includes external stint/orchestrator consumers, not only Rust
modules. Telemetry types stay out of terminal, retry, cleanup, merge-recovery,
and generic attention modules.

## 9. Capability enforcement and honest no-adapter behavior

Harness selection must evolve from a name-only registry to declared capabilities
and a resolved adapter probe. A harness can advertise protocol support only when
a trusted adapter helper is installed, compatible, enabled, and launchable.
Static `harness == "pi"` is not proof.

For an autonomous create:

1. resolve the selected harness;
2. require `worker_telemetry` capability;
3. run the adapter package's non-interactive `probe --output json` with closed
   stdin, a 2-second timeout, 4 KiB output cap, and no run mutation, then
   negotiate the highest mutually supported protocol version;
4. obtain a realpathed local extension entry, exact package version and integrity
   identity under an operator-configured trusted package root; reject writable
   or changed paths between probe and launch;
5. launch pi with ambient extension discovery disabled and that exact extension
   explicitly, rather than trusting settings or project-local packages;
6. pass the attempt capability path in the environment and scrub it from the
   adapter process environment after reading where the public runtime permits;
7. fail before worker materialization where possible; launch races/failures use
   existing told launch/worker-exit paths, never telemetry inference.

Interactive creation may proceed with `requirement: optional` and honest support
metadata. Claude has no real adapter in this design and must
be rejected for autonomous use; `--interactive --harness claude` remains honest.
No flag may claim support or silently downgrade an autonomous run. Every retry
revalidates the already-pinned probe/entry identity; if it disappeared or changed,
respawn fails through the existing typed retry/worker failure path while work is
preserved. A future Claude adapter must implement the same protocol and pass the
same conformance suite.

This enforcement should land together with or behind an explicit migration gate:
current users must not be surprised by a partially shipped state where
orchestratectl requires a package that does not yet exist. Human review chooses
the release transition.

## 10. External ownership and install trust

The pi adapter belongs in a separate repository and npm package, provisionally
`@jarimustonen/orchestratectl-pi-telemetry`. It contains:

- a pi extension using only documented `ExtensionAPI` events and `pi.exec`;
- a small probe executable that prints package version, supported protocol
  versions, and canonical extension entry path;
- protocol conformance and failure-injection tests.

The package declares `@earendil-works/pi-coding-agent` as a `peerDependency` and
runtime dependencies under `dependencies`, per pi package rules. Releases are
immutable, SemVer-versioned, provenance-attested, and pinned by exact npm version
(or immutable git commit during development). Orchestratectl never runs an
unpinned moving git ref and never auto-installs or auto-updates the package.
Installation is a deliberate user/admin action after source review. Pi warns
that packages and extensions execute with full system access; project-local
packages additionally require project trust. For autonomous orchestration, a
user-global pinned package plus orchestratectl's explicit resolved entry path is
preferred over repository-controlled `.pi` content.

The stint/orchestrator owns installation verification and deployment sequencing,
not individual workers. Doctor should report adapter absent, disabled, version
mismatch, entry-point drift, or capability-file permission failure, but must not
fix/install automatically.

## 11. pi.dev feasibility evidence

The installed 0.84.2 docs provide every required public hook:

- `session_start` and idempotent `session_shutdown` for timer ownership and
  incarnation opening/closing;
- `agent_start` and specifically `agent_settled`, documented as the point after
  automatic retry/compaction/follow-ups are exhausted;
- `tool_execution_start/update/end`, including parallel ordering semantics and
  `toolCallId`/`toolName`;
- `pi.exec(command, args, { signal, timeout })` for a public subprocess call;
- async handlers, with the important consequence that ordinary telemetry sends
  must be queued rather than awaited;
- the explicit rule to start timers only at `session_start`, not in the factory,
  and clean them in `session_shutdown`.

Two throwaway checks used only public APIs. First, a TypeScript extension
registered all hooks, tracked parallel tools, and owned a 30-second timer; the
public loader accepted it (`pi --no-extensions -e <file> --list-models`, exit 0).
Second, a live print-mode session queued `pi.exec` calls to a fake local endpoint:

```text
session_start → agent_start → agent_settled → session_shutdown
fake endpoint: settled, agent_active, settled, shutdown
session_shutdown awaited the queue and completed; pi exit: 0
```

This verifies that the installed runtime fires the required no-tool lifecycle,
queued subprocess sends survive handler return, and a bounded shutdown handler
can flush. It is not a production reliability or parallel-tool proof. It made no
orchestratectl writes and is not retained in the repository. Production work
still needs fake-endpoint parallel-tool, cancellation, timeout, reload, and
failure tests.
Relevant complete sources read were the installed `README.md`,
`docs/extensions.md`, `docs/packages.md`, `docs/environment-variables.md`, and
`examples/extensions/README.md`, plus the `status-line.ts`, `notify.ts`, and
`auto-commit-on-exit.ts` examples. The design does not use the documented
inter-extension `pi.events` bus because that would violate the harness-neutral
boundary.

## 12. Removed heuristics that must stay removed

Telemetry does not provide a new spelling for any deleted thin-supervisor
heuristic:

| Removed inference | Forbidden telemetry-era replacement |
|---|---|
| commit-time activity clock | Treating git changes as heartbeat/progress |
| tmux pane mtime/activity | Scraping pane output or using pane activity as state |
| CPU-rate clock | Sampling CPU to infer agent activity or wedging |
| idle-unmerged synthesizer | Converting stale/settled telemetry into failure or success |
| tmux tri-state/streak as primary liveness | Letting pane presence override told adapter facts |
| broad branch-is-ancestor reconciliation | Treating any telemetry state as evidence of merge/landing |
| kind-derived interactivity | Assuming pi/Claude or run kind implies lifecycle/capability |

Also forbidden: parsing pi session JSONL/logs, reading `PI_SESSION_FILE`, importing
an extension manager, consuming private EventBus events, or reaching into a
background-process extension. The adapter emits a fact through a neutral command;
orchestratectl does not guess it.

The typed terminal table remains exhaustive and telemetry-free. Cleanup continues
to consume only `TerminalOutcome::teardown` plus the existing source-relative,
dirty-worktree, and HEAD safety guards. A code review should reject any import of
telemetry into `TerminalOutcome::classify`, `TerminalOutcome::teardown`,
`cleanup_node`, retry eligibility, merge recovery, or the confirmed-death grace.

## 13. Failure-injection matrix

| Injection | Expected telemetry view | Status/cleanup invariant |
|---|---|---|
| Adapter crashes while pi lives | Current until 90s, then `stale` | No status change, retry, report, or teardown |
| Pi/launcher crashes | Lease becomes stale; told `worker.exited` or existing confirmed-dead grace handles terminal failure | Telemetry is not the crash verdict |
| orchestratectl endpoint unavailable | Adapter coalesces latest state and retries with capped backoff; lease may stale | Pi turn is not blocked; no fabricated state |
| Open committed, response lost | Same-ID retry returns the same epoch | No epoch churn |
| Old open completes after a new open | Last serialized open wins; displaced live instance reopens once or disables | No inferred outcome |
| Open accepted, first update absent | `awaiting_first_sample`; old sample ineligible | No stale state presented as current |
| Two same-ID opens | Idempotent | No epoch churn |
| Two different-ID opens | Serialized takeover; one current control fence | No mixed sample authority |
| Delayed heartbeat | Accepted only if epoch/seq are current; may restore `current` | No terminal effect |
| Duplicate heartbeat | Identical current epoch/seq/body is idempotent with no write | No duplicate history growth |
| Update committed, response lost | Exact immutable retry is idempotent | Sender does not mutate same sequence |
| Stale retry attempt | Secret/attempt mismatch rejected before write | New attempt remains authoritative |
| Old extension instance after reload | Old epoch rejected | New incarnation remains authoritative |
| Long healthy tool | `tool_running` refreshes every 30s; elapsed grows without history growth | Never timed out as failure |
| Clean `agent_settled` | Current `settled` while session lives | Still nonterminal; closing workflow required |
| `session_shutdown` on reload | Best-effort `shutdown`, then new `open` epoch and state | Shutdown is not completion |
| Event storm / parallel tools | Single-flight queue coalesces; bounded count/name | Write rate and file size remain bounded |
| Backward wall-clock jump | `clock_unreliable` | Unavailable only, never falsely terminal |
| Forward wall-clock jump | Early `stale` | Advisory degradation only |
| Malformed/oversized payload | Structured rejection, no write | Existing projection remains intact |
| Disk full between temp write/rename | Old complete projection survives; system error | No partial lease and no run-state mutation |
| Retry/terminalization races update/open | Authoritative generation/revocation wins | No post-close or old-attempt sample becomes current |
| Corrupt/missing sample | `invalid`/`absent`; updates refuse until open repairs | Control epoch never rolls back |
| Corrupt/missing control | Endpoint fails closed | No reconstruction from advisory sample |
| Endpoint lock held beyond deadline | Child is bounded/killed; later completion cannot evade fencing | Lease may stale only |
| >32 tools then count falls | Saturation flag sets/clears deterministically | State remains told, bounded |
| Duplicate/unmatched tool events | Deterministic adapter-local handling | No count underflow or unbounded wire state |
| Capability path substitution/symlink | Secure failure | No alternate identity accepted |
| Capability theft by another OS user | Prevented by owner-only directory/file | Same-user arbitrary code remains outside threat claim |

Tests must use fake clocks and a fake endpoint, strip ambient tools from `PATH`
where relevant, and assert negative invariants: no `node.report`, `run.status`,
retry, cleanup, or merge event is emitted by every telemetry failure case.

## 14. Ownership split and review checkpoints

### Orchestratectl repository

Owns protocol schema/version negotiation, capability issue/revoke and attempt
fencing, `node telemetry` CLI, bounded advisory projection, read surfaces,
harness capability enforcement, doctor diagnostics, and conformance fixtures.
This work touches correctness-sensitive state and must be sequenced: schema/event
identity first, endpoint/persistence second, read surfaces third, create/harness
enforcement last.

### External adapter repository

Owns pi public-event translation, timer/single-flight queue, privacy filtering,
probe executable, package release/install documentation, and cross-version pi
compatibility tests. It does not own durable run truth or background jobs.

### End-to-end lifecycle

Owns operator installation of the pinned package, integrated autonomous pi run
validation, explicit Claude-interactive behavior, and release migration timing.
Workers do not install packages or global binaries.

No production slice starts until a human accepts this design, especially the
capability-enforcement migration and external package ownership.

## 15. Independently reviewable implementation candidates

These are candidates for untriaged intake and human lane-or-close disposition;
none is authorized by this design alone:

1. **`worker-telemetry-core-control` — core telemetry identity and bounded projection** — protocol types,
   capability files/hash/generation, retry revocation, epoch/sequence reducer,
   atomic bounded lease file, injected-clock expiry, corruption behavior, and
   negative terminal/cleanup tests.
2. **`worker-telemetry-cli-surfaces` — public `node telemetry` endpoint and read surfaces** — strict open/update
   CLI/JSON contract, idempotency, `run show/list/wait` advisory views, text
   wording, snapshots, and write-amplification benchmark.
3. **`worker-telemetry-harness-enforcement` — harness capability enforcement and diagnostics** — adapter probe/version
   negotiation, explicit extension launch, autonomous rejection for unsupported
   harnesses (including Claude), interactive fallback, config/help/doctor, and
   migration gate.
4. **`worker-telemetry-pi-adapter` — external pi telemetry package** — separate repository/package, documented
   lifecycle hooks, privacy-preserving state machine, non-blocking coalescing
   sender, probe executable, pinned release/install trust, and adapter failure
   suite.
5. **`worker-telemetry-e2e-rollout` — end-to-end conformance and rollout** — install a pinned adapter in an
   isolated environment; inject every matrix failure; prove typed outcomes and
   work preservation are unchanged; document rollout/rollback and Claude manual
   behavior.

All five were filed through intake as `untriaged`, with this run's provenance,
for human lane-or-close disposition. The first three should be sequenced because
they overlap core schema and CLI DTO surfaces. The external package can begin
only after the protocol contract is human-approved and versioned. End-to-end
rollout depends on all prior slices.

## 16. Acceptance check

- Writes are authorized for the current node attempt and map only documented
  public lifecycle events; the protocol does not authenticate same-user adapter
  provenance.
- Silence is represented as missing/stale telemetry and has no terminal or
  teardown authority.
- Tool lifecycle and keepalive can be queued through public pi APIs without
  blocking ordinary turns; live no-tool lifecycle and shutdown flush were
  demonstrated, while parallel/failure behavior remains an implementation gate.
- Unsupported harnesses are explicit; autonomous Claude is rejected until a
  real adapter exists.
- No wire field is pi-specific, and no pi internal is imported or inspected.
- Identity, local authorization, bounded persistence, clocks, read surfaces,
  privacy, package trust, capability enforcement, and required failure
  injections are specified.
- Removed activity/merge/interactivity heuristics are explicitly barred, and
  the protocol explicitly does not diagnose wedging or progress.
- This phase contains design and feasibility evidence only.
