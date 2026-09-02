---
created: 2026-09-02
updated: 2026-09-02
type: feature
reporter: jari
status: in-progress
priority: normal
lane: rename
lane_seq: 10
collision: [repository-identity]
commits:
- hash: 7a20d7d
  summary: blocked ADR 0002 draft and implementation breakdown
---

# Rename orchestratectl to Taskfleet

## Description

## Goal

Rename the public project, CLI, Rust packages, distribution, configuration/state vocabulary, and documentation from `orchestratectl`/`octl` to **Taskfleet** with canonical command `taskfleet`, then converge dependent repositories through separately owned worktrees.

## Product decision

Jari selected Taskfleet after an international naming workshop. The canonical identities are intended to be:

- Product: Taskfleet
- GitHub repository: `jarimustonen/taskfleet`
- CLI command: `taskfleet`
- Primary crates.io package: `taskfleet`
- Homebrew formula/tap identity: `taskfleet`

The exact migration/compatibility contract must be decided before irreversible registry/repository changes.

## Phase 1 — migration decision

Record an ADR deciding how to migrate:

- the published `orchestratectl` and `octl-core` crates;
- binary invocation and scripts;
- `~/.orchestratectl` state/config and `ORCHESTRATECTL_*` environment variables;
- run manifests, JSON fields/schema compatibility, skills, docs, repository paths and symbols;
- GitHub repository rename and redirects;
- cargo-dist, crates.io, GitHub Release and Homebrew tap/formula;
- compatibility aliases, deprecation window, rollback, and release sequencing.

Compare at least a hard cut, a staged compatibility migration, and a packaging-only rebrand. Prefer the smallest safe path that makes Taskfleet canonical without losing existing run state or silently breaking automation.

## Phase 2 — current repository implementation

After the ADR lands, create a dependency-ordered implementation breakdown and complete the rename throughout this repository. Validate clean installation only through isolated/distribution paths; repository work must not mutate the user's installed binary or bundled instructions.

## Phase 3 — ecosystem convergence

Only after Taskfleet is published and verified, search other repositories for the old names and spawn one worktree in each owning repository. Do not globally replace generated/history/vendor files. In homebase/intake-related repositories, identify which repository/fleet unit owns the Haapa server before editing its configuration.

## Acceptance criteria

- Taskfleet is the canonical public name and `taskfleet` the canonical command.
- Existing durable run state and supported automation have an explicit safe migration or explicit documented break.
- crates.io, GitHub, releases, Homebrew and docs agree on identity.
- The old name remains only in intentional compatibility/migration/history contexts.
- Dependent repositories are discovered and converged by their own worktrees after the canonical release is live.

## Decisions

### 2026-09-02T06:06:15Z · @taskfleet-decision-worker

ADR 0002 remains Proposed, not Accepted: the required five-lens panel was incomplete because the Rust/crates.io release-engineering model returned no response. The useful draft and dependency-ordered plan are preserved on the technical-decision branch for a fresh complete-panel run; no rename or distribution action was taken.

### 2026-09-02T06:36:17Z · @taskfleet-decision-worker

ADR 0002 Accepted after a complete five-lens panel and cross-review. The decision chooses narrowed bounded compatibility: Taskfleet 0.6 directly handles 0.5.1 state; existing legacy homes are adopted in place until explicit quiescent migration; only the old CLI crate receives a bounded Cargo wrapper; new binary distribution is Taskfleet-only; Homebrew uses a new canonical tap plus a permanent old migration stub; and active compatibility sunsets at evidence/date/version-gated 0.8.0. DeepSeek returned HTTP 503 on the first maintainability lens; the one permitted fresh replacement position completed with gpt-5.6-sol. No rename, publication, repository/tap mutation, or installation occurred in this decision run.
