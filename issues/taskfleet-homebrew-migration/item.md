---
created: 2026-09-06
updated: 2026-09-06
type: task
status: done
priority: high
related: ['@taskfleet-release-0-6-0']
lane: taskfleet-rename
lane_seq: 120
collision: [jarimustonen/homebrew-orchestratectl, issues/taskfleet-distribution-topology/old-tap-migration]
closed: 2026-09-06
---

# Activate and verify Taskfleet Homebrew migration

## Goal

Execute ADR 0002 R11 only: activate the reviewed migration metadata in `jarimustonen/homebrew-orchestratectl`, then prove old and fresh Homebrew paths converge on the sole canonical `jarimustonen/taskfleet/taskfleet` formula without duplicate ownership or changes to the already-published Taskfleet channels.

## Preconditions

- R10 is closed in `issues/taskfleet-release-0-6-0`; read its terminal evidence under `evidence/final/`.
- v0.6.1 is the first fully published canonical release. Its formula is at canonical tap commit `c9e68594340b2b775d23159a3545d53f15306471`.
- The old tap must begin at exact head `85ce830378f38cf17283efddd966d5754354e403`.
- The reviewed patch is `issues/taskfleet-distribution-topology/old-tap-migration/0001-migrate-orchestratectl-formula-to-taskfleet-tap.patch`; its manifest records required/prepared trees.
- R10 authorizes only this R11 tap migration. Do not migrate dependent repositories, installed Taskfleet skills/binaries, or real user state here.

## Required work

- Revalidate the prepared old-tap patch against the exact old-tap head and its expected tree before publication.
- In the old tap repository, atomically publish the reviewed commit that deletes `Formula/orchestratectl.rb` and adds cross-tap migration metadata pointing `orchestratectl` to `jarimustonen/taskfleet/taskfleet`.
- Add metadata to the new tap only if a fully isolated recursive-resolution drill proves it necessary; otherwise leave canonical tap content unchanged except for cargo-dist's v0.6.1 formula.
- Run fresh canonical install, old receipt update/upgrade, `brew migrate`, old tap-qualified resolution, direct canonical install, and uninstall in fully disposable non-temporary Homebrew prefixes. Never use or modify the system Cellar, taps, formula cache, or real receipts.
- Verify only one canonical formula owns Taskfleet, no `orchestratectl` binary/alias is installed, version/embedded commit equal v0.6.1/`7e93bd6195fbaf6de0b43d9161228ae2373ab5d1`, and every disposable root is removed.
- If migration behavior is wrong, revert only the old-tap commit through a normal non-force push and leave Taskfleet crates/releases/tags untouched.
- Commit immutable sanitized receipts under this issue and close only when all paths converge.

## Acceptance Criteria

- [x] Old tap migration commit is based on exact reviewed head/tree and is publicly reachable.
- [x] Canonical tap remains the sole formula implementation and old tap contains migration metadata only.
- [x] Fresh canonical install and every supported old-receipt/migrate/qualified path resolve to `taskfleet` v0.6.1.
- [x] No old binary/alias, duplicate formula ownership, system Homebrew mutation, or disposable residue remains.
- [x] Checksummed evidence is committed and post-live dependent-repository discovery is the only next authorized phase.

## Resolution

### 2026-09-06T10:04:55Z · @issuectl

R11 passed against the actual public taps. Immutable checksummed evidence is under evidence/final/. This authorizes only ADR 0002 E1 post-live dependent-repository owner discovery; it does not authorize blind replacement, repository migration, deployment, installed skill/binary changes, or user-state migration.
