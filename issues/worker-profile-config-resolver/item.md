---
created: 2026-08-24
updated: 2026-08-24
type: task
status: open
priority: normal
provenance: other
provenance_detail: Approved worker control-plane implementation split
source_ref: orchestratectl:01m0sdkhhm5bxz7b9hvb7qxgt2/implementation-candidate:profile-resolver
originating_run: 01m0sdkhhm5bxz7b9hvb7qxgt2
originating_run_kind: spinoff
lane: worker-control-plane
lane_seq: 30
blocked_by: ['@worker-telemetry-cli-surfaces']
collision: [octl-core-schema, run-create, config-harness-selection, run-show-dto]
---

# Implement configurable agent profile resolver

## Description

Implement user-owned executable profiles and deterministic candidate resolution from the approved configurable-agent design.

## Scope

- Parse executable profile definitions only from `$ORCHESTRATECTL_HOME/config.toml`; reject executable definitions, commands, argv fragments, adapter paths, and residency changes in repository config.
- Validate bounded profiles with description, capability (`fast | capable | ultra-capable`), residency (`local | remote`), and up to eight ordered pi/Claude argv candidates with optional pi `telemetry = "worker-v1"`.
- Resolve the approved CLI, environment, repository-selection, and user-default precedence. Legacy harness inputs remain selection aliases for matching user-owned profiles and never synthesize commands.
- Own the complete deterministic pre-launch selection loop: evaluate `executable_missing`, `autonomous_harness_unsupported`, and `telemetry_unsupported` in that order; autonomous selection accepts only pi+`worker-v1`; fallback remains within the selected profile and cannot weaken residency or required telemetry.
- Preserve explicit interaction mode, keep post-selection launch/runtime failure out of fallback, and pin retry to the recorded candidate despite config or PATH changes.
- Record the compact requested profile/harness, selection source, interaction, selected candidate/harness, and one reason for each skipped candidate in dry-run, stored create data, and `run show`.
- Keep the existing user-owned local `secure` profile usable with normal user rights.

This issue is conservatively sequenced after `worker-telemetry-cli-surfaces` because both modify the core schema and per-node `run show` DTO; that edge is serialization, not a functional dependency.

## Acceptance criteria

- User/repository ownership, strict parsing, precedence, argv round trips, executable availability, unknown profiles, and conflicting same-level inputs are tested.
- Fallback reasons and selection match the exact candidate examined; failed launches do not advance fallback.
- Local requests never cross to remote, autonomous selection never chooses Claude or pi without `worker-v1`, and retry reuses the recorded candidate.
- Dry-run performs no run, worktree, pane, or telemetry mutation; legacy stored runs remain readable without invented history.
- No permission/operation model, trust grant, sandbox, package attestation, adapter probe, runtime-failure fallback, secret interpolation, automatic tier escalation, or raw model/effort CLI flags are added.

## References

- `issues/add-configurable-agent/design.md` §§2–6.
- `issues/worker-control-plane-review/integration-review.md` — approved profile/resolver split.
