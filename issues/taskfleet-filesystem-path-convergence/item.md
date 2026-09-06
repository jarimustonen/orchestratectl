---
created: 2026-09-06
updated: 2026-09-06
type: task
status: open
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 150
collision: [issues/rename-taskfleet/plan.md]
blocked_by: ['@taskfleet-zero-legacy-identity']
---

# Converge canonical Taskfleet filesystem paths

## Goal

Rename every maintained Taskfleet checkout, worktree root, project entry, registry target, and active persisted filesystem path to canonical `taskfleet` naming across all supported reachable machines.

## Ordering

This is a final convergence transaction. It depends on the zero-legacy Taskfleet release and every repository-level rename worktree. Do not begin while any Taskfleet-owned run, supervisor, worktree, tmux pane, intake job, or repository operation is active. Unreachable machines remain explicitly unverified and must converge when next reachable.

## Required work

- Inventory canonical source clones and worktree roots on every reachable host.
- Quiesce all Taskfleet operations and preserve unrelated dirty worktrees before renaming.
- Rename main checkout directories and `__worktrees` roots to canonical Taskfleet paths using Git-aware operations; repair `.git/worktrees` metadata and verify every registered worktree.
- Update Homebase tmux/workmux project declarations, source checkout provisioning, fleet checks, intakectl repository registry/anchor resolution, service paths, scripts, and any active persisted absolute path.
- Repoint every maintained Git remote to the canonical Taskfleet repository URL without relying on redirects.
- Handle state records according to the zero-legacy state contract; do not retain stale active path references under a former identity.
- Verify repository status, branch/remote identity, Taskfleet doctor/config/run reads, intakectl health, tmux/workmux discovery, and normal worktree create/merge in the renamed location.
- Remove empty obsolete directories and broken symlinks only after canonical replacements are independently verified.

## Acceptance Criteria

- [ ] Every reachable supported machine uses canonical Taskfleet clone and worktree-root directory names.
- [ ] Git worktree metadata, remotes, project launchers, fleet checks, and intake routing resolve only canonical paths.
- [ ] No active persisted path, obsolete directory, or broken symlink retains the former identity.
- [ ] One post-rename create/merge smoke test passes from the canonical checkout.
- [ ] Unreachable hosts are recorded as unverified rather than assumed converged.
