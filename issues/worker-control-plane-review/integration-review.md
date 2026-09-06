# Worker control-plane integration review

**Decision:** approved with simplifications on 2026-08-23; source designs revised
**Inputs:** [worker telemetry protocol](../worker-telemetry-protocol/design.md) and
[configurable agent profiles](../add-configurable-agent/design.md)
**Implementation:** proposal recorded below; filing and scheduling remain separate actions

## Recorded decision

Jari's binding decision from the owning issue is:

> Approved with simplifications on 2026-08-23. Binding product decisions: (1)
> remove the agent-permission/operation-set model; agents have full normal
> rights; (2) telemetry is a keep-it-simple advisory feature that tells the
> calling agent last reported activity and freshness so that caller can judge
> the situation—telemetry does not itself become success truth; (3) initially
> only pi with the adapter is autonomous, while Claude remains
> explicit-interactive; (4) fallback never weakens residency or telemetry
> requirements; (5) the existing local secure profile is usable now without
> special enforced restrictions, and tighter enforcement may come later; (6)
> executable agent commands live only in user-owned config; (7) requested and
> selected agent choice is plainly visible; (8) agent failure disclosure is
> accepted. Revise the source designs and implementation split to this simpler
> scope before implementation.

The revised designs treat this as a deletion decision, not as permission to
rename the rejected machinery.

## Approved product boundary

The combined v1 is small:

- A user-owned profile defines capability, residency, and an ordered list of
  agent commands. Repository config may only select a profile by name.
- Agents run with normal user rights. There are no permission sets, operation
  sets, restricted-local gates, tool filtering, command sandboxes, trusted
  launch contracts, or claims that a local profile is mechanically unable to
  spawn another agent.
- The existing local `secure` profile is usable. Its behavior comes from its
  configured model and instructions, not an taskfleet security boundary.
- Autonomous selection accepts only a user-configured pi candidate declaring the
  v1 adapter. Claude is eligible only when interaction is explicitly
  interactive until a real Claude adapter exists.
- Fallback stays within the selected profile and never weakens its residency or
  an autonomous telemetry requirement. Runtime failure does not advance
  fallback; retry keeps the recorded candidate.
- Output records the requested profile/harness, selected candidate/harness, and
  concise skipped-candidate reasons. It does not build a provenance graph.
- Telemetry stores one bounded last-told state plus server-stamped freshness.
  The calling agent may consider that evidence when deciding its next action.

The adapter remains harness-specific, but the update contract and read view are
harness-neutral. V1 deliberately omits package attestation, trusted roots,
probe executables, capability secrets, epoch/sequence fencing, and permission
brokering. Those mechanisms do not create a meaningful boundary against agents
that already have normal same-user rights.

## State-integrity boundary

Advisory does not mean authoritative. Telemetry cannot by itself:

- append or synthesize `node.report`;
- change node or manifest status;
- prove success, failure, merge, or landing;
- satisfy `run wait`;
- select or trigger retry; or
- classify a typed outcome or authorize cleanup.

`run merge` remains the only success truth. Told exits, cancellation, the fixed
confirmed-dead grace, merge transaction/recovery, typed outcomes, and existing
work-preservation guards remain the canonical non-telemetry paths.

This boundary still permits caller judgment. For example, a calling agent may
wait after a fresh `tool_running` observation or inspect a stale worker. Any
subsequent mutation must use an existing explicit command and obey that
command's ordinary rules; the reducer never acts on telemetry alone.

## End-to-end flows

### Autonomous pi

1. Resolve the requested user profile and interaction mode.
2. Preserve profile residency while walking its bounded candidate list.
3. Skip in deterministic order: `executable_missing`; then
   `autonomous_harness_unsupported` for non-pi; then `telemetry_unsupported` for
   pi without `worker-v1`.
4. Record and launch the selected user-owned command, supplying only exact
   run/node/attempt identity for the adapter.
5. The external pi adapter translates documented public lifecycle events into
   `agent_active`, `tool_running`, `settled`, or `shutdown`, sends state changes,
   and refreshes every 30 seconds.
6. Taskfleet validates the current attempt and atomically replaces one
   bounded sample. `run show` reports the last told state, age, and current/stale
   freshness using observational wording.
7. Completion and teardown continue through `run merge`, told failures, typed
   outcomes, and existing cleanup guards. Telemetry changes none of them.

A missing executable, adapter launch failure, or worker failure uses the accepted
existing failure disclosure. It does not trigger a more elaborate provenance or
fallback mechanism.

### Explicit-interactive Claude

1. Resolve `--interactive` and the selected user-owned Claude command.
2. Launch without claiming adapter support.
3. Show `requirement=optional`, `support=unsupported`, `sample=absent`.
4. Keep the run interactive until explicit `run merge` or `run cancel`; absent
   telemetry has no outcome or teardown meaning.

### Local `secure`

A user-defined local profile is selected and launched under the same rules. It is
not rejected for lacking a restricted operation set. Autonomous local use still
needs a pi+adapter candidate; interactive local use does not. Fallback cannot
leave the profile's local residency.

## Ownership

| Owner | Owns | Does not own |
|---|---|---|
| User config | Executable profile definitions, commands, adapter argv, declared capability/residency | Canonical run outcomes |
| Repository config | Profile-name selection only | Commands, adapter paths, executable definitions |
| Profile resolver | Precedence, bounded candidate order, residency/telemetry-preserving fallback, compact requested/selected output | Permissions, package trust, samples |
| External pi adapter | Public pi event translation, bounded in-memory tool tracking, coalescing/refresh, privacy filtering | Durable truth, outcomes, teardown |
| Taskfleet telemetry | Strict update DTO, current-attempt check, bounded atomic sample, freshness/read surfaces | Progress diagnosis, retry, settlement |
| Existing run control | Reports, told exits, merge transaction/recovery, typed outcomes, cleanup safety | Telemetry shortcuts |

## Post-decision assessment of existing telemetry candidates

At this design checkpoint these five items remained `untriaged`, unlaned, and
unchanged. The table recorded an implementation-candidate assessment for later
human disposition; it was not itself an acceptance, dependency edit, or
scheduling action.

| Candidate | Decision | Smallest scope under the simplified design |
|---|---|---|
| `worker-telemetry-core-control` | **Reshape** | Keep only strict run/node/current-attempt validation, one bounded atomically replaced sample, server timestamps/freshness, corruption handling, and negative reducer/outcome tests. Drop capability files, secrets, reducer-projected telemetry control, open/incarnation epochs, client sequences, and authorization claims. |
| `worker-telemetry-cli-surfaces` | **Reshape** | Implement one strict `node telemetry update` flags/JSON endpoint plus per-node `run show` and bounded `run list` freshness views. Drop `open`, capability-file transport, sequence idempotency, elaborate error-action classes, and any `run wait` integration. |
| `worker-telemetry-harness-enforcement` | **Reshape** | Reduce to static create-time eligibility: autonomous candidates must be user-configured pi+`worker-v1`; Claude is explicit-interactive; fallback preserves residency/telemetry; failures remain plainly disclosed. Drop trusted package roots, integrity/probe negotiation, ambient-extension suppression, launch attestation, and permission enforcement. |
| `worker-telemetry-pi-adapter` | **Reshape** | Keep a small external pi extension using documented events, sanitized four-state translation, 30/90-second refresh/freshness, single-flight coalescing, bounded shutdown, and fake-endpoint tests. Drop probe executables, package provenance requirements, open/reopen fencing, immutable sequence retries, and permission-aware launch integration. |
| `worker-telemetry-e2e-rollout` | **Reshape** | Validate autonomous pi, explicit-interactive Claude, local-profile behavior, stale/missing/corrupt samples, old attempts, event storms, clocks, endpoint/worker failure, and the negative outcome/cleanup invariants. Keep a modest subprocess/lock-load check and rollout notes; drop security-boundary, capability-path, package-integrity, and launch-attestation tests. |

## Post-approval implementation split

This is a proposal for independently reviewable slices, not issue filing or
scheduling. It leaves the five existing candidates' tracker metadata unchanged.

1. **Telemetry core** — reshape `worker-telemetry-core-control` to the bounded
   sample and current-attempt rules in the table above.
2. **Telemetry CLI/read surfaces** — reshape
   `worker-telemetry-cli-surfaces`; depends on telemetry core.
3. **Profile config and resolver** — a future, not-yet-filed profile candidate
   for user-only executable definitions, selection precedence, deterministic
   fallback, and compact requested/selected recording. It owns the first
   `run create` change and is sequenced before autonomous eligibility.
4. **Autonomous eligibility** — reshape
   `worker-telemetry-harness-enforcement`; depends on profile config/resolver and
   telemetry CLI/read surfaces. It is the second and only other `run create`
   slice, so the two cannot run in parallel.
5. **External pi adapter** — reshape `worker-telemetry-pi-adapter`; depends on
   the telemetry update endpoint and may proceed independently of the profile
   resolver once that contract is stable.
6. **End-to-end rollout** — reshape `worker-telemetry-e2e-rollout`; depends on
   all preceding slices and verifies the integrated boundary.

At this design checkpoint no profile issue had been filed. Human disposition,
lane assignment, and concrete dependency edits remained separate actions.

## Implementation disposition — 2026-08-24

The later planning action `reshape-worker-control-plane-dag` accepted and
reshaped the five candidates, filed `worker-profile-config-resolver`, and added
the conservative execution metadata. This status note does not alter the
recorded product decision above; the live issuectl DAG is the scheduling source
of truth.

## Review acceptance checklist

The source designs now agree that:

- agents have normal rights and no rejected permission model survives under a
  different name;
- telemetry is simple, told, fresh/stale, and useful to caller judgment without
  becoming reducer authority;
- only adapted pi is initially autonomous and Claude is explicit-interactive;
- fallback preserves residency and required telemetry;
- local `secure` is usable without enforced restrictions;
- executable definitions are user-owned and repository config is selection-only;
- requested/selected choice and fallback reason are plainly visible without
  elaborate provenance; and
- existing simple failure disclosure is sufficient.

Production code, bundled workflows, statuses of the five candidates, their lane
fields, and their dependency fields are outside this document change.
