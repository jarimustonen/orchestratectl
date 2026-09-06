# ADR 0002 — Taskfleet has one identity

- **Status:** Superseded and amended
- **Date:** 2026-09-02
- **Amended:** 2026-09-06
- **Decider:** Jari Mustonen
- **Issue:** `rename-taskfleet`

## Context

Taskfleet was introduced through a staged rename plan. That plan retained a
second package and command, dual state/config resolution, input aliases, state
movement tooling, and distribution transition artifacts. The resulting product
had two operational identities and substantially more state-sensitive code.

The maintainer superseded that strategy on 2026-09-06. Published registry
artifacts and git objects are immutable external history, but maintained source
must not encode or operate a second identity.

## Decision

Taskfleet uses a clean-break identity:

- the product, repository, Cargo packages, command, release assets, formula,
  diagnostics, telemetry, prompts, and skills use Taskfleet names;
- the workspace contains exactly `taskfleet-core` and `taskfleet`;
- the only executable is `taskfleet`;
- state and user configuration live only under `~/.taskfleet`, selected only by
  `TASKFLEET_HOME`;
- repository configuration is only `.taskfleet.toml`;
- branded environment, worker, notification, test, and internal protocol
  variables use the `TASKFLEET_*` namespace;
- worker telemetry identifies the Taskfleet adapter contract;
- the release topology publishes two crates in dependency order, followed by
  the independent Taskfleet GitHub Release and Homebrew legs;
- maintained code contains no wrapper, alias resolver, state adopter/mover,
  warning, migration receipt, installer redirect, tap transition, or renamed
  skill mover.

Neutral durable schema vocabulary remains unchanged where it is not an identity.
This decision does not rewrite user state, git history, tags, or registry data.
Operators and separately maintained repositories converge independently outside
this repository change.

## Consequences

There is one implementation and one operational source of truth. Existing data
or automation under any other identity is not discovered or modified. This is a
breaking release and must be described as such.

The release gate requires:

1. a tracked-file path and content search with zero retired identity references;
2. workspace metadata and package archives containing only the two canonical
   packages and one executable;
3. the full Rust, snapshot, stripped-environment, release-policy, and
   distribution gates;
4. no release, tag, publish, install, state movement, tap mutation, or external
   repository mutation during implementation.

## Rejected alternative

A compatibility window was implemented and then rejected. Keeping a second
identity, even temporarily, conflicts with the maintainer's requirement that
maintained HEAD describe and execute Taskfleet only.
