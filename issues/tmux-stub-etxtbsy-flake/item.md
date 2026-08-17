---
created: 2026-08-16
updated: 2026-08-17
type: bug
status: open
priority: high
lane: multiplexer
lane_seq: 10
---

# Flaky test: tmux stub hits ETXTBSY on Linux CI

## Description

CI on `main` is red. Run
[31964603146](https://github.com/jarimustonen/orchestratectl/actions/runs/31964603146)
(commit `febc554`, the v0.2.1 release commit) failed on `ubuntu-latest`:

```
thread 'multiplexer::tmux::tests::new_session_surfaces_nonzero' panicked at
crates/octl-cli/src/multiplexer/tmux.rs:495:22:
expected NonZero, got Spawn { op: "new-session", source:
  Os { code: 26, kind: ExecutableFileBusy, message: "Text file busy" } }

test result: FAILED. 445 passed; 1 failed
```

## Root cause

The test writes a stub `tmux` script to a temp dir, marks it executable, and then
spawns it. On Linux the write handle is still open when `execve` runs, so the
kernel returns `ETXTBSY` instead of running the script. The test then sees a
`Spawn` error where it asserted a `NonZero` exit. This is a race, not a logic
bug: it does not reproduce on macOS (which does not enforce ETXTBSY the same
way) and it passes on reruns, which is why earlier `main` runs are green.

## Fix

In the stub-writing helper, close the file before spawning: take the
`std::fs::File`, `sync_all()` it, and `drop()` it (or scope the write in its own
block) *before* setting the permission bits and invoking the binary. Writing to
a temp name and `rename`-ing into place also removes the window.

Since the same helper backs every `tmux` stub test, fixing it once covers the
whole module and removes a class of nondeterministic CI failures.

## Impact

Flaky red CI on `main`. The v0.2.1 release itself is fine — `Release` and
`Publish to crates.io` both succeeded and the
[v0.2.1 release](https://github.com/jarimustonen/orchestratectl/releases/tag/v0.2.1)
exists — so this is a test-harness defect, not a shipped regression.
