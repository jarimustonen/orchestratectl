---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: jari
status: open
priority: normal
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 10
collision: [repository-identity]
---

# Freeze Taskfleet rename identity inventory

## Description

## Goal

Execute R0 of `issues/rename-taskfleet/plan.md`: freeze the complete identity inventory and sanitized orchestratectl 0.5.1 compatibility fixtures before any source/package rename.

## Scope

- Inventory packages/binaries, public branded environment/config, stable `OCTL_*` protocol variables, state/config paths, subprocess self-exec, skills/prompts/provenance, release scripts/workflows, URLs/actions, cargo-dist assets, Homebrew tap/formula ownership, generated/history/vendor boundaries, and external consumers visible from authoritative metadata.
- Classify every old-name occurrence as active identity, bounded compatibility, permanent safety/history, test fixture, generated/vendor, or external convergence.
- Capture sanitized 0.5.1 homes/fixtures for completed, non-terminal, pending-merge, config/profile, installed-skill provenance, and unknown-but-readable state schema values. Preserve event bytes and avoid user-global state.
- Recheck `taskfleet`, `taskfleet-core`, `jarimustonen/taskfleet`, and tap/formula availability without treating a check as reservation.
- Materialize the ADR plan as dependency-ordered implementation issues/scheduling if needed, but do not start R1 or mutate public identities.

## Acceptance criteria

- No identity-bearing writer or distribution surface is unidentified.
- Fixtures validate using an isolated published 0.5.1 artifact/environment and are safe to commit.
- Every retained old-name category has an explicit migration owner.
- No source/package/repository/tap rename, publication, global install, or user-state mutation occurs.
- `git diff --check`, fixture checks, and `issuectl doctor --json` pass.
