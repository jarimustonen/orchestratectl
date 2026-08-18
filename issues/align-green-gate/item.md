---
created: 2026-08-18
updated: 2026-08-18
type: task
status: done
priority: normal
closed: 2026-08-18
---

# Align green gate with CI

## Description

Align worker verification guidance with the actual CI commands and prohibit global orchestratectl installs from worktrees.

## Acceptance Criteria

- [x] Root and bundled-worker guidance use the locked release-mode CI gate.
- [x] Workers use worktree-local release binaries and never install orchestratectl globally.
- [x] Orchestrator deploy verification compares the installed binary's commit to `HEAD`.

## Resolution

### 2026-08-18T05:11:39Z · @issuectl

Aligned the documented and bundled worker gate with locked release-mode CI commands, prohibited global worker installs, and added commit-identity deploy verification.
