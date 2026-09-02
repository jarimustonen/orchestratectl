---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: done
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R5
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 60
blocked_by: ['@taskfleet-package-wrapper']
collision: [repository-identity]
closed: 2026-09-02
commits:
- hash: 03d8782
  summary: 'feat: migrate workflows and contracts to Taskfleet'
---

# Convert Taskfleet skills, prompts, provenance and repository contracts

## Description

## Goal

Make new generated commands/source refs use `taskfleet`; rename only Taskfleet-owned skill identities while keeping generic workflow skill names. Migrate Claude/Codex markers and pi schema-v3 provenance by recorded hashes, preserving edited/user-owned files and readable old records. Update AGENTS, README, architecture/security/contribution docs, examples, templates, telemetry prose and `OSS-RELEASE.md`; retain stable `OCTL_*` protocols and classify every residual old identity.

**Acceptance:** full insta review loop; isolated install/update/prune/orphan/provenance tests include unchanged, edited, unmanaged, stale, corrupt and partial old/new ownership while preserving user bytes; generated prompt headings/commands close via the exact canonical run id; telemetry contract id `orchestratectl.worker-telemetry-adapter` and stable `OCTL_*` remain unchanged; skill example extraction validates canonical commands instead of silently finding zero; classified case-insensitive search has no unexplained active old name; no global skill install or distribution mutation.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `60`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-package-wrapper`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.

## Resolution

### 2026-09-02T16:54:20Z · @issuectl

Completed ADR 0002 R5. Canonical Taskfleet skills, generated prompts/source refs, docs, examples, telemetry availability prose, and OSS contracts now use taskfleet while bounded orchestratectl compatibility, OCTL_* vocabulary, telemetry contract id, pre-cut guards, and user-owned bytes remain preserved. Full Rust green gate, package checks, contract validation, issue doctor, residual classifier, and multi-model review/assessment passed.
