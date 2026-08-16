---
created: 2026-08-16
updated: 2026-08-16
type: bug
status: open
priority: normal
related: ['@run-create-long-title-stillborn']
lane: lifecycle
lane_seq: 25
---

# Use ULID entropy in run-create branch names

## Description

## Description

`run::create::derive_branch_name` uses the first 10 characters of a ULID. Those characters encode the millisecond timestamp, not random entropy. Two runs created in the same millisecond with titles sharing the retained slug prefix can therefore derive the same `wt/<short-id>-<slug>` branch name and cause `workmux add` / git branch creation to fail.

This predates the long-title stillborn fix, but that fix makes the shared-prefix case more visible by bounding the title slug to the workmux-compatible window length.

## Suggested direction

Preserve a compact random suffix from the ULID in the branch identifier while retaining the bounded total name length. Decide deliberately whether compatibility with existing visible branch naming is worth a migration or whether only new creates need the stronger identifier.

## Reproduction

Use two generated ULIDs with identical first ten timestamp characters and long titles that normalize to the same bounded slug prefix. Both currently derive the same branch name.
