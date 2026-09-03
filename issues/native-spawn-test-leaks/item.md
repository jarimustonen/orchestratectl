---
created: 2026-09-03
updated: 2026-09-03
type: bug
reporter: jari
status: fixed
priority: normal
related: ['@taskfleet-native-materialization']
lane: taskfleet-rename
lane_seq: 86
collision: [repository-identity]
closed: 2026-09-03
closed_by: agent
---

# Keep native spawn validation out of real worktrees

## Observed occurrence

Validation for `@taskfleet-native-materialization` left 14 clean worktrees/branches and 16 tmux fixture windows in the real orchestratectl repository/headless session after reporting success. Names included `smoke`, `interrupted`, `live-child`, `doomed-child`, and `pending-coordinates`. One real-pi smoke branch, `wt/9htzpmpwzn-live`, contains two unmerged alternative implementation commits (`d06608a`, `fa2acef`), so it was preserved while all proven-empty fixtures were safely removed.

## Impact

The native spawn implementation passed its functional gate but its validation polluted the user's real git worktree registry and shared tmux session. A test also handed production task context to a live agent, creating substantial duplicate work. This violates the disposable-test and cleanup acceptance boundary and makes a green suite unsafe to run.

## Goal

Make every native materialization/integration test hermetic and self-cleaning, including successful, failed, interrupted, parent/child and real-agent smoke paths.

## Acceptance Criteria

- [x] Tests use temporary repositories and isolated tmux sockets/sessions; no fixture is registered under the developer's source repository or shared `headless` session.
- [x] Cleanup runs on success, assertion failure, timeout, interruption and child-process failure, preserving only genuinely unmerged fixture work inside its disposable temp root.
- [x] Live smoke uses a harmless bounded prompt and cannot execute the issue implementation task or merge into a real branch.
- [x] A before/after inventory assertion proves zero new worktrees, branches, tmux windows, supervisors and run roots outside the declared sandbox.
- [x] The preserved `wt/9htzpmpwzn-live` commits are reviewed against landed `f7193a1`/`391ebe0`; retain useful differences explicitly or record why the branch can be deleted safely.
- [x] Full native spawn and Rust green gates pass without residue.
- [x] No global install, publication or user-state mutation.

## Residue and alternative-branch assessment

The residue had two sources. The fixture names (`smoke`, `interrupted`, `live-child`, `doomed-child`, and `pending-coordinates`) map directly to the alternative implementation's integration-test scenarios, which ran materialization from the real checkout and used the shared tmux server. The later `native-pi-smoke` window came from a separate ad-hoc live-agent check that reused the implementation prompt. Both validation paths were unsafe; the committed production implementation did not create this residue during ordinary use.

`git range-diff`, `git cherry`, and a source/test diff were used to compare preserved commits `d06608a`/`fa2acef` with landed `f7193a1`/`391ebe0`. The branches are independent implementations of the same issue rather than cherry-pick equivalents, but the preserved version has no unique correctness improvement to retain. The landed implementation has the stronger attempt-bound JSON handshake and start identity, exact pane/socket identity, profile-selection and retry integration, publication transaction, and broader rollback tests. The alternative's large create-script-era test suite is the source of the real-repository/shared-tmux pollution addressed here; `fa2acef` only closes the already-closed parent issue with less validation detail than `391ebe0`. The preserved branch is therefore reviewed and superseded, not silently discarded.

## Validation

- Native creation, all-kinds, and end-to-end spinoff suites passed repeatedly with byte-for-byte identical before/after source-worktree, `wt/*` ref, shared-tmux, and test-supervisor inventories.
- The same native test binaries passed with `PATH=/usr/bin:/bin`.
- The full green gate passed: `cargo fmt --all --check`, workspace clippy with warnings denied, 1,110 release nextest tests, release doctests, and rustdoc with warnings denied.
- `cargo package --locked --workspace --no-verify` packaged all three crates from a clean tree.
- The checked real-pi smoke passed twice using a disposable repository/home, harmless no-tools prompt, private tmux socket/session, exact generated prompt-archive cleanup inside the sandbox, bounded cancellation, and an unchanged external inventory.
- The smoke's fail-closed inventory assertion also correctly rejected one attempt when an unrelated Vensum tmux window appeared concurrently; the check was not weakened.
- Reviewed alternative branch `wt/9htzpmpwzn-live` was deleted only after the supersession assessment above. No production branch/worktree preservation behavior was weakened.

## Resolution

### 2026-09-03T10:47:45Z · @agent

Native spawn validation now runs in owned disposable repositories and private tmux state, cleans candidates/supervisors on every fixture path, and has a checked bounded real-pi smoke. Repeated native, stripped-PATH, full Rust, package, and external-inventory gates passed with no residue; the reviewed alternative branch was superseded and removed.
