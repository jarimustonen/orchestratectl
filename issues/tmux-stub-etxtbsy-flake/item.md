---
created: 2026-08-16
updated: 2026-08-17
type: bug
status: in-progress
priority: high
lane: multiplexer
lane_seq: 10
commits:
- hash: 1104fb1042099e4ccbfe1614155fe2d959a04209
  summary: 'fix: close tmux test stub before exec'
- hash: e822897
  summary: 'fix: close tmux test stub before exec'
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

## Reopen Notes — 2026-08-17

_Add rationale for reopening here._

## Comments

### 2026-08-17T07:32:35Z · @orchestrator

Reopened 2026-08-17: the first fix (sync_all + drop before chmod in `fake_tmux`) was necessary but NOT sufficient. Main CI on 1566b5e still fails on ubuntu with ETXTBSY (`ExecutableFileBusy`, os code 26), now in TWO sibling tests: `new_session_headless_returns_pane_id_and_disables_rename` (direct ETXTBSY) and `find_window_by_path_scopes_to_all_when_no_session` (downstream symptom: the stub spawn fails so the probe returns None instead of Some("@9")). Each test already uses its own tempdir, so this is NOT path sharing between tests. Leading hypothesis: a cross-thread fork/exec race. Rust runs these tests as parallel threads in ONE process; when thread A still holds (or has just held) a write fd to its stub, thread B's Command::spawn forks and the child transiently inherits that fd. CLOEXEC closes it only at exec, so between fork and exec a live process holds a write handle to A's stub, and A's own exec then gets ETXTBSY. A loaded CI runner widens that window, which is why it is Linux-only, load-dependent, and survived the close-before-chmod fix.

### 2026-08-17T07:35:41Z · @worker

Confirmed the cross-thread fork/exec race by tracing the shared in-process Rust test model and `Command::spawn` mechanics: a fork can inherit another fake stub writer before CLOEXEC takes effect at exec, producing Linux ETXTBSY despite closing the original writer before its own spawn. Chose a test-local mutex held from `fake_tmux` creation through each fixture lifetime, so every fake-tmux write and every command it executes are serialized. This covers the entire fake_tmux family without changing production code.
