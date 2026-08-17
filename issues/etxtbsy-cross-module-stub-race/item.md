---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: in-progress
priority: high
lane: lifecycle
lane_seq: 2
commits:
- hash: b1be65d109c5cc1a27810dca7e683674bd85fe46
  summary: start ETXTBSY cross-module race fix
---

# ETXTBSY flake is cross-module: all executable-stub test fixtures share one fork/exec race, tmux-only mutex insufficient

## Description

## Symptom

Main CI red on f6a2feb (run 32041657942): `multiplexer::tmux::tests::new_session_surfaces_nonzero` failed on `test (ubuntu-latest)` with `expected NonZero, got Spawn { op: "new-session", source: Os { code: 26, kind: ExecutableFileBusy } }` — even though the test uses the `fake_tmux` fixture whose `FAKE_TMUX_SERIAL` mutex (the `tmux-stub-etxtbsy-flake` fix, stint 4) is held for the whole test.

## Root cause

The stint-4 mutex serializes only the **fake-tmux family against itself**. The unit-test binary runs ALL `src/` module tests as threads in ONE process, and at least four OTHER modules also create executable stub files (`from_mode(0o755)`): `run/merge.rs`, `git/repo.rs`, `supervise/capture.rs`, `supervise/cleanup.rs` — none of them takes the tmux mutex. The race is process-wide: while ANY thread holds a write fd on its stub, ANY other thread's `Command::spawn` forks a child that transiently inherits that fd (O_CLOEXEC closes it only at exec); if the stub is exec'd while such a pre-exec child still holds the inherited fd, Linux returns ETXTBSY. So a merge.rs/repo.rs stub write racing a fake-tmux spawn reproduces exactly the failure the tmux-local mutex was supposed to close.

## Expected

A structural, process-wide fix for the whole class — not another per-module mutex. Candidates (worker decides, argue on mechanism):
- One process-global stub-write vs. spawn exclusion shared by ALL stub-writing fixtures (e.g. a global RwLock in a shared test-support module: stub creation holds write, test spawns hold read), replacing/absorbing `FAKE_TMUX_SERIAL`.
- Or run CI tests with cargo-nextest (process-per-test eliminates the class structurally) — bigger CI change, weigh it.
- A bounded ETXTBSY retry is at most defense-in-depth, not the fix.

Verify on mechanism; a green local run is NOT evidence for this class (macOS does not enforce ETXTBSY; CI is the only gate that sees it).

## Context

Blocks the v0.3.0 tag (CI-green-on-tagged-commit precondition). Same class as `tmux-stub-etxtbsy-flake` (stint 4, two spinoffs); this is the third occurrence — the cross-module scope was the missed half. The cargo-deny failure on the same run was an unrelated GitHub 429 transient.
