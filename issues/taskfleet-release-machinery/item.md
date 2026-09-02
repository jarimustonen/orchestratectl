---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: done
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R6
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 70
blocked_by: ['@taskfleet-package-wrapper', '@taskfleet-skills-docs-contracts']
collision: [repository-identity]
closed: 2026-09-02
commits:
- hash: 5ad76787b0fed382dc70e4e4cc3b2425a29eb517
  summary: rebuild Taskfleet release saga
- hash: 1e73c33c48981af216c4483e099a81ddec31108a
  summary: test registry reconciliation failures
- hash: 4b00a1c8b22178b5aea3b8212f1cd324e07a2237
  summary: pin five-leg topology
- hash: 3cb1a6de5eaafe0e0035b15a3688e6eaa67aa2dc
  summary: refresh identity inventory
- hash: 70a58da33d5db9350d79e7c8276715c71652c905
  summary: harden release mutation boundaries
- hash: aba9f5abc155b8aca3d1d937e84ca8b20b902d74
  summary: reconcile exact registry receipts
- hash: b9242cc51a02e7fbd73275d100ac42dfece8ee52
  summary: format release assertions
- hash: 03ac9fb32a383f8f04e068f3769b560327ce1c8f
  summary: diagnose held-journal mismatches
- hash: f5f26302af815ac4b2698ecce36193c474c8d1f7
  summary: pin packaging and transient retries
- hash: 241dd752033db2ac2faa01c6a6d08557dc3feb52
  summary: refresh final identity ledger
- hash: f1a315a58426e237c58c364f4e5eba12e2e2efcc
  summary: admit Shipshape journal schema six
- hash: 0440909c1c697eec18bca86e192ce6b54d13c397
  summary: parse mixed resume diagnostics
- hash: 769c5460efb17e2138303e876692ec17d11666bb
  summary: preserve failed protocol fixtures on request
- hash: 13c29d5f0f2243110037140220c2be60cf6629a1
  summary: pin five-target verify outcomes
---

# Rebuild Taskfleet registry and Shipshape release machinery

## Description

## Goal

Replace the hard-coded two-crate workflow with `taskfleet-core` → `taskfleet` → `orchestratectl`, waiting for each exact dependency to become index-visible. Reconcile an existing package/version only after checksum, owners, dependency requirements, metadata and source commit match. Make repository/package identities in the pinned Shipshape 0.10.1 wrapper data-driven while preserving held-tag exact-SHA gates, deterministic version hooks and protocol tests. Document partial-success resume/fix-forward and Homebrew empty-commit repair.

**Acceptance:** package archives and sealed dry-run plan contain exactly three intended crates legs and independent distribution legs; exact pins/version snapshots pass; all release-wrapper protocol tests pass; the current error-text inference that “already exists” means success is removed in favor of checksum/owner/dependency/metadata/source receipts; side-effecting tools are stubbed and credentials absent in pre-cut tests; no local publish, tag, GitHub rename, tap change or install.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `70`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-package-wrapper`, `@taskfleet-skills-docs-contracts`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.

## Agent Runs

### 2026-09-02T18:46:46Z · @taskfleet-r6-worker

Completed ADR 0002 R6 release machinery. The release topology is now one strict data-driven five-leg contract: ordered crates.io `taskfleet-core` → `taskfleet` → bounded `orchestratectl`, plus independently observed cargo-dist GitHub Release and Homebrew targets. The CI-only publisher enforces the activated exact GitHub tag/repository/SHA context, pins Cargo 1.98.0 for byte-stable resumes, waits/retries authoritative index propagation without interpreting Cargo diagnostics, and reconciles non-yanked archive checksum, complete owners, full dependency semantics, version metadata, and `.cargo_vcs_info.json` source commit into per-leg receipts. Shipshape's exact 0.10.1 held-tag, main fast-forward, exact-SHA CI, resume, verify, bump-hook, and journal gates remain intact and are data-driven from `release/taskfleet-release.json`.

R7 remains deliberately blocked (`activation: blocked-r7`) and still owns cargo-dist/new-tap regeneration; no tag, publish, install, repository rename, tap mutation, or real registry mutation occurred. Partial success/fix-forward and generated Homebrew empty-commit repair are documented. Multi-model review plus `/assess-findings` confirmed and drove the HTTP/API/activation/dependency/toolchain/test hardening; the only deferred review item is already owned by `@taskfleet-distribution-topology` (R7), so no duplicate issue was filed.

## Acceptance Criteria

- [x] Exact three-crate package graph and five-leg sealed Shipshape plan verified.
- [x] Registry reconciliation verifies checksum, owners, full dependencies, metadata, non-yanked state, and source receipt without parsing Cargo success text.
- [x] Held-tag, interrupted/resume, mismatch, partial-success, package archive, snapshot, workflow, contract, audit, Rust, and issue-doctor gates pass.
- [x] R7 activation remains blocked and no publish, tag, repository/tap mutation, install, credential use, or real registry mutation occurred.
