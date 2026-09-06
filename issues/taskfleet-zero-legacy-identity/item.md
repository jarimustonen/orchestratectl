---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 140
collision: [Cargo.toml, crates, docs]
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

## Acceptance criteria

- [ ] ADR 0002 and the rename plan define the clean break.
- [ ] The workspace contains only `taskfleet-core` and `taskfleet`, and ships
  only the `taskfleet` executable.
- [ ] State/config resolution uses only `TASKFLEET_HOME`, `~/.taskfleet`, and
  `.taskfleet.toml`.
- [ ] Branded environment, worker/notification, tracing, telemetry, fixtures,
  prompts, and skills use Taskfleet identities.
- [ ] No secondary dispatcher, resolver, mover, alias, warning, receipt, stub,
  tap transition, or renamed-skill migration remains.
- [ ] Release policy publishes the two canonical crates and Taskfleet-only
  GitHub/Homebrew artifacts.
- [ ] A strict tracked-path/content inventory reports zero retired identity
  references.
- [ ] Snapshot review, full green gate, release/distribution tests, stripped
  environment tests, and package inspection pass.

## Safety boundary

This implementation does not release, tag, publish, install, move real state,
mutate a tap, or edit another repository.
