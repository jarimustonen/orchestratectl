---
created: 2026-09-06
updated: 2026-09-06
type: task
status: done
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 140
collision: [Cargo.toml, crates, docs]
closed: 2026-09-06
closed_by: pi
commits:
- hash: a357c0c
  summary: make Taskfleet the sole identity
---

# Keep only the Taskfleet identity

## Goal

Make maintained HEAD Taskfleet-only across packages, commands, state,
configuration, environment, protocols, telemetry, skills, documentation, tests,
and release/distribution contracts.

## Binding decision

The maintainer selected a clean break on 2026-09-06 and superseded ADR 0002's
staged strategy. Immutable external registry artifacts and git history are out of
scope; maintained tracked source must contain only Taskfleet identity.

## Acceptance Criteria

- [x] ADR 0002 and the rename plan define the clean break.
- [x] The workspace contains only `taskfleet-core` and `taskfleet`, and ships
  only the `taskfleet` executable.
- [x] State/config resolution uses only `TASKFLEET_HOME`, `~/.taskfleet`, and
  `.taskfleet.toml`.
- [x] Branded environment, worker/notification, tracing, telemetry, fixtures,
  prompts, and skills use Taskfleet identities.
- [x] No secondary dispatcher, resolver, mover, alias, warning, receipt, stub,
  tap transition, or renamed-skill migration remains.
- [x] Release policy publishes the two canonical crates and Taskfleet-only
  GitHub/Homebrew artifacts.
- [x] A strict tracked-path/content inventory reports zero retired identity
  references.
- [x] Snapshot review, full green gate, release/distribution tests, stripped
  environment tests, and package inspection pass.

## Safety boundary

This implementation does not release, tag, publish, install, move real state,
mutate a tap, or edit another repository.

## Resolution

### 2026-09-06T14:25:10Z · @pi

Completed and verified by taskfleet run 01m1ve2w3jsrsze85nxqa0edps. Full green, stripped-environment, package, distribution, release-topology, and exact Shipshape 0.10.1 protocol gates passed; production remote remained untouched.
