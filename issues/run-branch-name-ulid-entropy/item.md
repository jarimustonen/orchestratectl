---
created: 2026-08-16
updated: 2026-08-20
type: bug
status: fixed
priority: normal
related: ['@run-create-long-title-stillborn']
lane: lifecycle
lane_seq: 5
closed: 2026-08-20
commits:
- hash: 418231c
  summary: fix worker branch ULID entropy
---

# Use ULID entropy in run-create branch names

## Description

`run::create::derive_branch_name` uses the first 10 characters of a ULID. Those characters encode the millisecond timestamp, not random entropy. Two runs created in the same millisecond with titles sharing the retained slug prefix can therefore derive the same `wt/<short-id>-<slug>` branch name and cause `workmux add` / git branch creation to fail.

This predates the long-title stillborn fix, but that fix makes the shared-prefix case more visible by bounding the title slug to the workmux-compatible window length.

## Suggested direction

Preserve a compact random suffix from the ULID in the branch identifier while retaining the bounded total name length. Decide deliberately whether compatibility with existing visible branch naming is worth a migration or whether only new creates need the stronger identifier.

## Reproduction

Use two generated ULIDs with identical first ten timestamp characters and long titles that normalize to the same bounded slug prefix. Both currently derive the same branch name.

## Decisions

### 2026-08-20T08:50:07Z · @pi-agent

Use the final 10 ULID characters (50 randomness bits) for new branch display identifiers. Preserve the existing 10-character identifier and 36-character retained slug budgets so the 50-byte workmux window bound and long-title behavior stay unchanged. Do not migrate existing branches or projections: exact ownership uses the recorded canonical worktree path plus branch. Accepted visible trade-offs: the new identifier is not a run-id prefix and does not preserve chronological branch sorting.

## Acceptance Criteria

- [x] Same-millisecond ULIDs with the same retained title slug derive distinct bounded branch names.
- [x] Existing long-title workmux bounds and separator behavior remain unchanged.
- [x] Legacy and entropy branch formats resolve only through exact recorded ownership.

## Resolution

### 2026-08-20T09:14:30Z · @issuectl

New branches now retain 50 ULID randomness bits while preserving the workmux length budget; deterministic branch and legacy/new exact-ownership tests pass.
