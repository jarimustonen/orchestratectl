---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: untriaged
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R6
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
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
