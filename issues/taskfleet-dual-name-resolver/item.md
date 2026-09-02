---
created: 2026-09-02
updated: 2026-09-02
type: task
reporter: taskfleet-r0-worker
status: open
priority: normal
provenance: other
provenance_detail: ADR 0002 planned implementation DAG
source_ref: orchestratectl:01m1gdgp1hqt1aa2fdpq8q5hqs/planned-dag:R2
originating_run: 01m1gdgp1hqt1aa2fdpq8q5hqs
originating_run_kind: spinoff
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 30
blocked_by: ['@taskfleet-shared-dispatcher']
collision: [repository-identity]
---

# Add the Taskfleet dual-name resolver and legacy-home adoption

## Description

## Goal

Implement the ADR home/config/input matrix from the frozen 0.5.1 fixtures. Add canonical `TASKFLEET_HOME`, `TASKFLEET_PROFILE`, `TASKFLEET_HARNESS`, `TASKFLEET_LOG` and `.taskfleet.toml`; retain old branded aliases/fallback through 0.7 with old-only/equal warnings and differing-value refusal. With no explicit home, use canonical-only, adopt legacy-only in place, create fresh canonical when neither is populated, and refuse dual-populated roots. Route logs, doctor, config, skills/provenance, subprocesses and every command through this one resolver. Preserve all `OCTL_*` spellings.

**Acceptance:** define managed/populated roots, lexical/path equivalence, case sensitivity, relative/nonexistent paths, inaccessible and symlink roots, and explicit-home split-root behavior; exhaustive environment/home/repository-config matrix; normalized equivalent paths accepted with warning; split truth refuses reads requiring one root and every write. Resolver/conflict selection occurs before logging or any filesystem write; help is filesystem-pure, conflict warnings are stderr-only/once per top-level invocation/JSONL-safe, and hidden self-exec children do not repeat them. A published 0.5.1 process and the new reader/writer interoperate on one adopted legacy root under the documented operator-exclusion limit. Fixture state/config/provenance bytes do not change (logs are isolated); no physical movement or source/package rename.

## Intended scheduling (human disposition required)

- Related parent: `@rename-taskfleet` (the parent remains unscheduled)
- Intended lane: `taskfleet-rename`
- Intended lane sequence: `30`
- Intended collision: `repository-identity`
- Intended blocked by: `@taskfleet-shared-dispatcher`
- This worker filed the issue unlaned/untriaged as required by run policy. An authorized human must accept it and apply the exact scheduling metadata; do not spawn it before that disposition.
