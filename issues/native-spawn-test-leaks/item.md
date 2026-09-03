---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: open
priority: normal
related: ['@taskfleet-native-materialization']
lane: taskfleet-rename
lane_seq: 86
collision: [repository-identity]
---

# Keep native spawn validation out of real worktrees

## Observed occurrence

Validation for `@taskfleet-native-materialization` left 14 clean worktrees/branches and 16 tmux fixture windows in the real orchestratectl repository/headless session after reporting success. Names included `smoke`, `interrupted`, `live-child`, `doomed-child`, and `pending-coordinates`. One real-pi smoke branch, `wt/9htzpmpwzn-live`, contains two unmerged alternative implementation commits (`d06608a`, `fa2acef`), so it was preserved while all proven-empty fixtures were safely removed.

## Impact

The native spawn implementation passed its functional gate but its validation polluted the user's real git worktree registry and shared tmux session. A test also handed production task context to a live agent, creating substantial duplicate work. This violates the disposable-test and cleanup acceptance boundary and makes a green suite unsafe to run.

## Goal

Make every native materialization/integration test hermetic and self-cleaning, including successful, failed, interrupted, parent/child and real-agent smoke paths.

## Acceptance criteria

- Tests use temporary repositories and isolated tmux sockets/sessions; no fixture is registered under the developer's source repository or shared `headless` session.
- Cleanup runs on success, assertion failure, timeout, interruption and child-process failure, preserving only genuinely unmerged fixture work inside its disposable temp root.
- Live smoke uses a harmless bounded prompt and cannot execute the issue implementation task or merge into a real branch.
- A before/after inventory assertion proves zero new worktrees, branches, tmux windows, supervisors and run roots outside the declared sandbox.
- The preserved `wt/9htzpmpwzn-live` commits are reviewed against landed `f7193a1`/`391ebe0`; retain useful differences explicitly or record why the branch can be deleted safely.
- Full native spawn and Rust green gates pass without residue.
- No global install, publication or user-state mutation.
