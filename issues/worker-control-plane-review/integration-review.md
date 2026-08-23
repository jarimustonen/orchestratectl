# Worker control-plane integration review

**Checkpoint:** review requested; no human decision or production authorization is
recorded here.

**Inputs:** [worker telemetry protocol](../worker-telemetry-protocol/design.md) and
[configurable agent profiles](../add-configurable-agent/design.md).

## Recommendation

Approve the shared boundary with the amendments and conservative v1 answers
below. This is a recommendation, not Jari's approval.

The coherent core is:

- capability, declared residency, interaction, permission requirement,
  telemetry requirement, and probed support remain separate dimensions;
- autonomous launch requires a trusted compatible telemetry adapter and any
  required permission-enforcement evidence; fallback never weakens a constraint;
- pi with the separately packaged adapter is the only expected initial
  autonomous harness; Claude remains available through explicit interactive
  selection without fabricated telemetry;
- create records requested and effective policy with field-level provenance;
  retry reuses that policy and candidate instead of re-resolving or advancing;
- telemetry is advisory. It cannot supply progress, success, failure, retry,
  `run wait`, merge, terminal-outcome, or teardown truth.

For initial remote profiles, the recommended interpretation of `standard` is the
existing unrestricted model-visible operation surface: it requires trusted
launch-plan binding, but no restrictive operation-set enforcement. A
`restricted-local` candidate requires separate mechanical enforcement and stays
ineligible until that boundary is approved and demonstrated.

## Cross-design reconciliation

The designs do not duplicate terminal or policy truth, but approval should carry
these amendments back to the source documents:

1. **Launch composition is not telemetry-adapter work.** Correct configurable
   profiles §10: the adapter retains all telemetry §14 responsibilities—public pi
   event translation, adapter-local tool state, timer/single-flight queue,
   privacy filtering, probe implementation and compatibility output, packaging,
   and lease sending. It does not establish its own trust identity, compose the
   final harness launch, enforce permissions, or own durable/terminal truth.
   Jari must choose whether harness-specific composition lives directly in
   orchestratectl or behind a separately versioned launch contract.
2. **Trust precedes execution.** Reorder telemetry §9 steps 3–4: authenticate the
   operator-owned registry entry, probe executable/package, trusted root,
   version/integrity, and harness binding before executing the bounded probe.
   Probe output establishes compatibility only. Revalidate the same identity
   before launch.
3. **Optional adapters receive authority too.** Telemetry §4 currently describes
   capability issue only for telemetry-required attempts, while profiles §5/§6
   correctly issue it whenever a compatible adapter is selected, including
   interactive pi. Key issuance to selected adapter support, not to `required`.
4. **Legacy output is a public-contract decision.** Pre-gate records cannot be
   relabeled supported or unsupported. Recommended representation: a versioned
   `policy: {recording: "legacy-unrecorded"}` variant, no invented telemetry
   requirement/support, `sample=absent`, and a separate
   `telemetry_policy_unrecorded_nodes` count; exclude those nodes from required
   and supported denominators. This amends `run show/list` JSON, not the
   open/update wire protocol. Jari must confirm the shape with grandfathering.
5. **Environment cleanup must preserve reload semantics.** Telemetry §9's
   best-effort capability-path scrubbing cannot erase the only path needed for
   §4's required `open` on later pi `session_start` reload/new/resume/fork.
   Retain attempt access for the pi process unless a public runtime mechanism can
   both hide it from model-visible children and make it available to each new
   adapter incarnation. Do not promise an unsafe or impossible scrub.
6. **Use precise terminal wording.** `run merge` is the only success truth, not
   the only terminal path. Told failing `worker.exited`, the fixed-grace
   confirmed-dead backstop, cancellation, and typed non-success outcomes remain
   independent of telemetry and interaction mode.

### State and fencing

- Requested/effective policy and provenance are immutable create-time facts.
- Per-attempt revalidation evidence is appended; it never rewrites create-time
  policy or support evidence.
- Telemetry control—attempt, generation, capability hash, revocation, current
  client and epoch—is authoritative run state under existing lock/reducer rules.
- The accepted sample, including accepted client sequence/body used for
  idempotency and conflict detection, is a replaceable projection. Its loss or
  corruption never reconstructs control and requires a new open where specified.
- The capability file and bearer lifetime are orchestratectl-owned,
  attempt-scoped material. Only the adapter's in-memory secret copy is
  disposable. Adapter tool map, timer, queue, and next-sequence allocator are
  incarnation-local.
- Attempt/generation/secret fence retries; client/epoch/sequence fence pi
  incarnations. A takeover clears old-sample eligibility.
- Reports, told exits, merge transactions/recovery, typed outcomes, and cleanup
  guards remain the only settlement and teardown inputs.

### Constraints and retained risks

Fallback is create-time only. It never crosses profiles, local→remote,
autonomous-required→unsupported, or stricter→weaker permissions. Runtime failure
and retry never advance the chain. Repository config cannot define executable
profiles, adapters, trust grants, residency, permissions, or support.

The design does **not** sandbox arbitrary same-user code, prove local network
confinement or model quality, eliminate the final same-user check-to-exec race,
or make pi extensions less than full-user-permission code. Dry-run executes an
authenticated external probe and is no-run/no-mutation, not globally
side-effect-free. Requiring the one pinned adapter makes autonomous availability
sensitive to package/runtime/entry drift; hard-pinned retries may require a new
run. Interactive Claude remains diagnostically opaque to this telemetry surface.
The CLI endpoint also costs about 120 unchanged refresh subprocesses per
hour/node, with 1,800 accepted updates/hour/node only as the saturated ceiling;
contention and fsync benchmarks remain rollout gates.

## End-to-end flows

### Autonomous pi with the external adapter

1. `run create` derives autonomous lifecycle from the absence of explicit
   `--interactive`, resolves profile constraints and `telemetry=required`, and
   records every selected, derived, conflicting, and shadowed input.
2. Resolution checks at most the bounded candidate set. It preserves profile
   residency and permissions, authenticates adapter/executable identity before
   probe execution, applies per-probe plus aggregate count/output/time budgets,
   and records selected/skipped/untried reasons. Exhaustion fails before worker
   materialization.
3. The first fully eligible pi candidate becomes an immutable effective policy
   and final launch plan. `standard` binds the existing unrestricted surface;
   restricted sets additionally require verified composition. Identity is
   revalidated immediately before materialization.
4. Orchestratectl creates owner-only attempt authority outside the worktree and
   launches the exact pi executable with ambient extension discovery disabled
   and the exact adapter entry enabled. Capability creation failure aborts before
   materialization. Failure after creation revokes/removes authority, records the
   existing typed launch-aborted/failure path, preserves work, and never tries a
   later candidate.
5. At each `session_start`, the adapter opens an incarnation. Public pi events
   feed its bounded local state; a single-flight queue sends state changes and
   30-second refreshes. Under current attempt/client/epoch fencing,
   orchestratectl accepts higher sequences, idempotently acknowledges an
   identical repeated sequence/body, and rejects conflicts or stale writers. A
   takeover exposes `awaiting_first_sample`, never the old sample as current.
6. `run show` presents immutable policy/provenance, separate per-attempt evidence,
   and telemetry `requirement/support/sample`. `settled`, `shutdown`, stale,
   absent, invalid, or clock-unreliable observations leave status unchanged.
   `run wait` ignores telemetry.
7. Successful completion is established only by `run merge` and its recorded
   transaction/report. Told failing exit or the confirmed-dead grace produces
   existing typed crash outcomes. Typed outcome and cleanup safety code alone
   decide teardown; terminalization revokes attempt authority.
8. Retry preflights the recorded executable, adapter, protocol, and permissions
   without holding the run lock across subprocesses. Under lock it compares the
   attempt/status snapshot, increments the absolute attempt, durably revokes old
   authority, rotates generation and secret, issues a new capability, and spawns
   the same recorded plan. Post-transition failure is a typed failed launch and
   never revives old authority.

### Explicit-interactive Claude without telemetry

1. `--interactive --harness claude` (or an equivalent eligible profile) records
   interaction from the CLI; harness never implies lifecycle.
2. Telemetry is optional. Claude has no adapter, so it is eligible only for this
   interactive path with `support=unsupported`; no probe result, capability, or
   sample is fabricated.
3. Orchestratectl freezes and launches the ordinary Claude base command without
   mutating global settings. `run show` records the effective policy and displays
   `requirement=optional, support=unsupported, sample=absent`.
4. Unsupported/absent telemetry and pane loss alone are not success, failure, or
   teardown authority. Success still requires `run merge`; a told failing
   `worker.exited` or the existing confirmed-dead backstop may produce the
   existing typed crash outcome independently of telemetry. Existing explicit
   cancellation and work-preservation behavior is unchanged.

## Ownership

| Owner | Owns | Must not own |
|---|---|---|
| External pi adapter | Public-event translation, local tool state, timer/queue, privacy filtering, probe implementation/output, package compatibility, lease sender | Trust establishment, permissions, durable run truth, outcomes, background jobs |
| Harness-neutral orchestratectl telemetry protocol | DTO/version negotiation, capability issue/revoke, fencing, bounded sample, read surfaces, doctor diagnostics, conformance fixtures | pi internals, progress/wedge inference, settlement |
| Profile/config resolver | Trusted definition/selection layers, orthogonal constraints, candidate eligibility/fallback, requested/effective policy and provenance | Samples, retry inference, repo executable definitions |
| Harness launch-composition boundary (owner is a Jari decision) | Canonical executable binding, final argv/environment, ambient extension policy, model-visible operation set, permission evidence | Event interpretation, terminal outcomes, repository trust grants |
| Durable run state | Immutable policy, per-attempt evidence, authoritative telemetry control, existing event projections | Adapter queues or bearer secret in public policy |
| Existing merge/terminal paths | Told exit, merge transaction/recovery, explicit report, typed outcome table, cleanup safety | Profile or telemetry shortcuts |
| Later end-to-end stint lifecycle | Deliberate adapter installation, integrated validation, rollout/rollback and migration operation | Worker auto-install; new protocol semantics |

## Decision for Jari

Choose **approve recommended answers**, **approve with named amendments**, or
**reject/return for redesign**. Approval must identify the selected answers; the
recommendation alone records no decision.

| Decision routed here | Recommended v1 answer | Consequence if deferred or changed |
|---|---|---|
| 1. Restricted-local operation set and feasibility | Keep `secure` visible but ineligible; define the useful closed operation set and accept enforcement evidence before launch | Local profiles remain unavailable; allowing them without evidence weakens the stated boundary |
| 2. Launch-composition ownership | Orchestratectl-owned harness boundary, separate from telemetry | A separate contract adds version/release coordination and must be designed first |
| 3. Adapter registry/trust | User/operator-owned registry under `$ORCHESTRATECTL_HOME`; owner-only definitions; authenticated root/probe/package/entry/harness binding before probe; exact integrity identity | No autonomous candidate can be trusted until these semantics are fixed |
| 4. Repository authority and precedence | Disable repo selection in v1; a present repo profile key fails `repo_selection_not_authorized`; retain CLI/env/user selection | Repo convenience waits; enabling it requires a grant/ceiling model and confirmation of specificity-first precedence |
| 5. Reserved roles and aliases | Reserve `ultra-capable`, `capable`, `fast`, `secure`; fixed semantic metadata, replaceable candidate lists; do not retain `expert`/`standard`/`implementer` aliases in v1 | Alias compatibility or different replacement rules require a documented mapping |
| 6. Optional telemetry | Deterministically attach a selected trusted compatible adapter; otherwise unrestricted interactive launch proceeds honestly without telemetry | Making optional probe failures fatal would reduce interactive availability |
| 7. Retry and migration transaction | Hard-pin/fail-on-drift; confirm prepare/commit ordering above; grandfather pre-gate retry behavior; require explicit future re-resolution; use one atomic release gate after adapter availability | A temporary feature gate needs explicit diagnostics, rollback and a removal release; no gate may falsely label unsupported workers eligible |
| 8. Public launch metadata and credentials | Persist unredacted base/final argv and package integrity as public run metadata; exclude capability material/private diagnostics; no profile secret interpolation | Explainability wins over argv secrecy; users must keep credentials out of argv, or v1 needs a separately designed secret-reference/redaction contract |
| 9. Built-in fleet mapping | Vendor-neutral remote capability roles using current harness aliases; `secure` is an unavailable local placeholder, not a confinement claim | Vendor-specific defaults leak fleet policy; no universal local candidate exists |
| 10. Raw model/effort escape hatch | Omit `--model`/`--effort` in v1 | One-off vendor selection waits for a concrete need and equivalent provenance rules |
| 11. Legacy read shape and autonomous enforcement premise | Use the versioned legacy representation above; accept that trusted telemetry is an observability eligibility gate despite remaining outcome-advisory | A different shape affects JSON consumers; weakening the gate would require redesign, not fallback |

The operation set, registry write/trusted-root semantics, aggregate resolver
budget, retry/grandfathering behavior, and legacy public shape are approval-time
contract decisions. Capability-file protections and telemetry v1 limits already
specified by the telemetry design are normative. Internal module layout, private
DTO organization, and test organization may remain evidence-driven implementation
choices.

**Material tradeoffs:** the recommendation maximizes explainability and prevents
silent policy weakening, but removes autonomous Claude after migration, leaves
local and repo-selected profiles unavailable, makes the pinned adapter an
availability dependency, exposes user-supplied argv in durable output, and makes
identity-drifted runs non-retryable without a new run.

**Approval would authorize only:** record the chosen amendments in the source
designs; consider each existing telemetry candidate through its separate human
lane-or-close disposition; and define/file new profile candidates only after the
approved ownership and trust decisions are recorded. Approval does not itself
accept, lane, schedule, start, install, enable, or release anything.

## Smallest safe post-approval dependency shape

If and only if later candidates are separately accepted and scheduled, the
smallest technical dependency shape is: freeze protocol/trust fixtures → prove
core fencing and advisory persistence → expose the bounded endpoint/read contract
→ prove the external adapter against that contract → prove trusted launch
eligibility → add profile/config resolution and provenance → validate rollout and
migration end to end. This is dependency guidance, not a schedule or scope
assignment to an existing issue; shared schema, reducer, `run create`, retry, and
CLI DTO surfaces must be sequenced.

The five existing items remain untriaged and unchanged:
`worker-telemetry-core-control`, `worker-telemetry-cli-surfaces`,
`worker-telemetry-harness-enforcement`, `worker-telemetry-pi-adapter`, and
`worker-telemetry-e2e-rollout`. This review neither accepts nor expands them and
creates no profile implementation slice.
