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

# Remove all legacy identity from Taskfleet

## Goal

Complete Taskfleet as a clean rename with zero maintained legacy identity: remove every active or compatibility reference to the former product from Taskfleet source, packages, commands, state/config, environment, protocols, skills, docs, tests, and current issue/decision artifacts.

## Owner directive

The maintainer explicitly superseded ADR 0002's bounded-compatibility strategy on 2026-09-06: do not retain or refer to the old name anywhere. All legacy references are to be removed, and every dependent repository is to receive a comprehensive repository-level rename worktree rather than a narrow occurrence patch.

Already-published registry versions, immutable git history, and externally immutable release/tag receipts cannot be erased. They are not maintained HEAD surfaces. Everything editable at maintained HEAD is in scope, including migration documentation/evidence whose only purpose is the retired compatibility strategy.

## Required work

- Amend/supersede ADR 0002 and `issues/rename-taskfleet/plan.md` so the repository's binding design matches this directive; remove the compatibility-window/C1-C3 strategy.
- Remove the `orchestratectl` Cargo wrapper package/binary and every old package/crate/repository/Homebrew/installer coordinate from maintained HEAD.
- Remove legacy command aliases, config/env aliases, legacy-home adoption and migration code, warnings, receipts, split-root logic, and tests. Canonical state/config is only `~/.taskfleet` and `TASKFLEET_*`.
- Rename stable-but-old-branded `OCTL_*` worker/notify variables and `orchestratectl.worker-telemetry-adapter` to Taskfleet identities, updating schemas, prompts, skills, tests, docs, and control-plane contracts consistently.
- Rename old-branded source directories/modules/test fixtures where needed (including `octl-*` identities) so a tracked-source search has no maintained old identity.
- Delete or rewrite current migration issues/evidence/docs that preserve the former identity. Do not rewrite git history or attempt to delete immutable published artifacts/tags.
- Update release/distribution contracts for Taskfleet-only publication. The next release must not publish the wrapper or old binary/assets.
- Run the full repository green gate, snapshot review loop, distribution/release-policy tests, and stripped-PATH tool-sensitive tests.
- Do not release, install, migrate real user state, mutate Homebase/other repositories, or alter public taps/tags from this worktree. Those are separately ordered conductor actions.

## Gate

A tracked HEAD search (excluding only `.git`, build output, and immutable git object history) must return zero old product-name/package/URL/home/protocol references. Any unavoidable external immutable receipt must be listed outside maintained source, not encoded as compatibility behavior.

## Acceptance Criteria

- [ ] Binding ADR/plan reflect clean-break zero-legacy policy.
- [ ] Taskfleet builds and tests with only canonical package/binary/state/env/protocol/skill identities.
- [ ] No compatibility wrapper, alias, legacy resolver/migrator, old protocol id, or old-branded directory remains.
- [ ] Full green gate and snapshots pass.
- [ ] Exact zero-reference inventory is recorded and the repository is ready for a Taskfleet-only breaking release.
