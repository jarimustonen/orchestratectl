---
created: 2026-08-15
updated: 2026-08-15
type: task
status: done
priority: high
epic: lifecycle-architecture-review
commits:
- hash: d7b5599
  summary: 'feat(run): add explicit --interactive how-run flag'
- hash: e23712a
  summary: 'fix(run): apply /llm-review findings to interactive flag'
closed: 2026-08-15
---

# Thin supervisor: add explicit interactive flag

## Description

## Goal
Add the explicit `--interactive` how-run flag from the 0.2 design, replacing the removed kind-derived interactive lifecycle.

## Context
Interactivity is no longer a run kind. The flag means the supervisor never auto-terminalizes or auto-tears-down; it waits for explicit `run merge` or `run cancel`, and a dead pid alone is not a terminal condition. Per the design, pi.dev is the default harness; claude remains reachable only as an interactive opt-in.

Likely touches `run create`, schema/state carried in the manifest, harness selection, supervise semantics, docs, and bundled skills that spawn runs.

## Done criteria
- `taskfleet run create --interactive ...` persists explicit how-run state.
- Supervisor semantics match the design: no auto terminalization/teardown from pid death alone in interactive mode.
- CLI docs/snapshots and bundled skills are updated.
- Full project green gate and relevant insta snapshot loop pass, including docs.
