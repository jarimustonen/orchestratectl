# Worker control-plane rollout

This document covers the simplified worker control plane split between this
repository and the separately owned pi.dev adapter package.

## Validated repository boundary

Taskfleet validates its side of the adapter boundary only through the public
v1 contract in [`contracts/worker-telemetry-v1/`](../contracts/worker-telemetry-v1/).
The test fake is an endpoint driver: it submits caller-supplied request bytes to the exact
published `taskfleet node telemetry update --input-file - --output json`
argv. It contains no pi hooks, lifecycle translation, timers, coalescing,
shutdown callback, extension-manager access, or production adapter code.

The integrated evidence is intentionally layered:

| Obligation | Evidence |
|---|---|
| Autonomous configured pi+`worker-v1`, deterministic fallback, local `secure`, explicit-interactive Claude, dry-run/create/stored `run show` | `crates/taskfleet/tests/run.rs`: `profile_resolution_matches_dry_run_persisted_create_and_show`, `telemetry_requirement_and_support_come_only_from_recorded_policy`, `exhausted_profile_returns_full_compact_attempt_without_mutation` |
| Exact selected argv and exported run/node/absolute-attempt identity under a stripped ambient `PATH` | `crates/taskfleet/tests/run.rs`: `materialized_create_routes_through_the_recorded_exact_argv`; `crates/taskfleet/src/supervise/mod.rs`: `profile_retry_uses_recorded_candidate_and_absolute_attempt` |
| Launch failure stays a disclosed launch failure; retry and idempotent replay keep the recorded candidate despite config/PATH drift | `profile_launch_failure_stays_a_launch_failure_without_publication_or_fallback`, `idempotent_profile_replay_uses_recorded_selection_after_config_drift`, and the supervisor retry test above |
| Strict current/old-attempt/malformed endpoint behavior and accepted `agent_active`, `tool_running`, `settled`, and `shutdown` samples | `crates/taskfleet/tests/telemetry_contract.rs` and `crates/taskfleet/tests/node.rs` |
| Missing, current, stale, corrupt, malformed stored data, old attempts, backward/forward clock effects, and expiry overflow | `crates/taskfleet-core/tests/telemetry.rs` with an injected clock |
| Long tools, refresh, event storms, single-flight coalescing, endpoint failure, settled/shutdown, and bounded shutdown | Published harness-neutral virtual traces in `conformance.json`; repository tests validate trace consistency and submit every accepted reference payload to the real public endpoint |
| Retry/terminal races and modest lock/subprocess load | `terminal_and_retry_races_serialize_with_telemetry_validation`, `concurrent_replacement_never_exposes_partial_or_mixed_samples`, and endpoint fixture subprocess execution |
| Telemetry cannot report, change status, select retry, satisfy wait, prove merge/landing, classify outcomes, authorize cleanup, or delete work | Every endpoint fixture and accepted trace inventories all non-telemetry run files, permits exactly the named advisory sample, preserves the full simulated worktree, and checks public `run show`/`run wait`; core negative tests inspect all canonical control inputs. Existing `e2e_spinoff`, supervisor outcome, merge, and cleanup suites independently exercise the authoritative paths without importing telemetry. |

Telemetry sample files are replaceable advisory projections. Corruption and
unavailability remain localized to telemetry warnings/views; they never repair
or advance canonical state.

## External package status

**The production pi.dev adapter package is not present in this repository or in
the available installed pi package set. It was not installed, executed, or
validated during this rollout.** Repository trace-oracle tests are not a claim
that real pi hooks, scheduler timing, package installation, or shutdown
callbacks passed.

Delivery is tracked as the unlaned intake item
`uncommonly-vague-family` (“Deliver the external pi worker-telemetry adapter
package”), filed from the rollout run. That package must consume the published
fixtures and demonstrate an ordinary install in an isolated temporary home and
package root. It must not mutate user-global pi settings, package state, the
installed taskfleet binary, or installed taskfleet skills.

## Rollout order and failure disclosure

1. Release taskfleet with the endpoint, profile resolver, recorded launcher,
   read surfaces, and this conformance suite.
2. Publish the external adapter package separately. Test it against the minimum
   supported taskfleet contract in an isolated environment.
3. Operators install the adapter by their package's ordinary mechanism and add
   its invocation to a user-owned pi candidate with `telemetry = "worker-v1"`.
   Repository config may select that profile but cannot define executable argv.
4. Start with explicit-interactive runs, verify accepted samples and observational
   `run show` wording, then enable autonomous selection.

Selection-time missing executables fail with the recorded bounded fallback
reason. Post-selection launch, adapter, endpoint, and worker failures use the
existing plain failure disclosure and never advance fallback. Endpoint/adapter
failure leaves the prior sample to age to `stale`; absent or stale telemetry is
not converted into failure, retry, completion, or cleanup permission.

## Rollback

Rollback does not rewrite durable history:

1. Stop selecting the affected profile (or select an explicit-interactive Claude
   profile) in user/repository selection config.
2. Let existing runs finish through `run merge`/`run cancel`; retry remains pinned
   to each run's recorded candidate and must not be used as a profile migration.
3. Remove or disable the external adapter using that package's own documented
   mechanism. Do not edit taskfleet run projections or telemetry samples.
4. If necessary, roll back taskfleet through its normal distribution
   channel. Old runs without selection metadata remain readable as
   `legacy-unrecorded`; advisory telemetry files can be ignored by older builds.

No rollback step auto-installs, uninstalls, attests, probes, or edits global pi
extensions. Package trust and lifecycle remain operator responsibilities.
