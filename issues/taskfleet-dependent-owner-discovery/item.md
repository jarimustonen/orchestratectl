---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: high
related: ['@taskfleet-homebrew-migration']
lane: taskfleet-rename
lane_seq: 130
collision: [issues/rename-taskfleet/plan.md]
---

# Map dependent Taskfleet owners and active references

## Goal

Produce ADR 0002 E1's evidence-backed owner map for every maintained active Taskfleet dependency before any cross-repository convergence work begins.

## Scope

Search maintained repositories and Homebase fleet configuration for active references to:

- `orchestratectl` / `taskfleet` commands and package names;
- canonical and legacy repository URLs, actions, installers, Homebrew taps/formulae, and Cargo coordinates;
- `ORCHESTRATECTL_*`, `TASKFLEET_*`, `OCTL_*`, `~/.orchestratectl`, and `~/.taskfleet` configuration/state;
- installed bundled skills and prompt paths;
- `orchestratectl.worker-telemetry-adapter` and pi runtime integration;
- fleet units, launchers, release-secret setup, Haapa machine configuration, and intake-related deployment.

Use repository AGENTS files, git-tracked searches, `homebase fleet status/doctor` or their read-only equivalents, and reachable-machine status. Exclude `.git`, targets/builds, vendor/generated caches, worktrees, immutable release evidence, archived history, and intentional compatibility fixtures from active findings; classify each retained legacy occurrence explicitly.

## Deliverable

Write `issues/taskfleet-dependent-owner-discovery/owner-map.md` with one row per owning repository/unit containing:

- owner repository and exact paths/units;
- active consumer or dependency purpose;
- command/package/channel currently used;
- state/config location and compatibility constraints;
- machine scope/reachability, especially Haapa;
- required ordering and prerequisites;
- proposed focused E2 worktree issue slug;
- references that must remain intentionally unchanged.

Record which repository owns Haapa and intake deployment based on evidence, not inference. End with a disjoint/serialized E2 wave plan and an E3 search baseline. Do not change any dependent repository, machine, installed binary/skill, state, secret, tap, or Taskfleet source in E1.

## Acceptance Criteria

- [ ] All maintained reachable repositories and Homebase fleet units are covered by an auditable search inventory.
- [ ] Active references are separated from intentional compatibility/history/generated occurrences.
- [ ] Haapa and intake ownership is identified with exact repository paths/units and reachability status.
- [ ] Each required E2 change has one owning repository, dependency channel, ordering, and proposed issue.
- [ ] E1 authorizes only the listed E2 worktrees; no migration has occurred.
