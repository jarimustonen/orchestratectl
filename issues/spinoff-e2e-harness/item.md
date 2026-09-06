---
created: 2026-06-28
updated: 2026-06-29
type: feature
status: fixed
priority: normal
closed: 2026-06-29
---

# End-to-end test harness for autonomous-spinoff full lifecycle

## Description

Feature: build an end-to-end test harness that exercises the full autonomous-spinoff lifecycle (spawn → work → merge → terminal node.report → auto-cleanup) in an isolated environment — separate git repo, separate `~/.taskfleet/` root, separate tmux session, separate workmux config — so the harness can run in CI and locally without polluting the developer's live repo or live tmux.

The need surfaced 2026-06-28: the supervisor terminal-cleanup spinoff (run 01kw7e3brb...) had to do its end-to-end live smoke verification by hand-crafting a TestHome temp dir and reaching around workmux/tmux/git's normal coupling. That worked once for one test, but it's not reproducible: the next developer who wants to verify the full loop has to re-derive the workaround. A standing harness encodes the recipe.

Goals:

1. Run a real `taskfleet run create --kind spinoff` against a real (throwaway) git repo and a real (throwaway) `~/.taskfleet/` root.
2. The spinoff agent in the test does a deterministic trivial task ("touch /tmp/smoke-marker-$$ && exit 0" or similar) so the test asserts on observable side effects without depending on a real LLM call.
3. Verify the full loop:
   - run reaches lifecycle `done`
   - supervisor process exits on its own
   - tmux window is gone (or never existed if --headless)
   - worktree directory is removed
   - branch is deleted
   - no leaked orphan processes
4. Run in `cargo test` as an integration test, gated by a feature flag (e.g. `e2e-spinoff`) because it requires real tmux + real git + real workmux on PATH.
5. Be reproducible: same setup, same teardown, idempotent re-runs.

Implementation sketch:

- New file: `crates/taskfleet-cli/tests/e2e_spinoff.rs` (or a separate `crates/taskfleet-e2e/`).
- Test fixture:
  - `tempfile::TempDir` for throwaway git repo (`git init`, an initial commit, a `main` branch).
  - `tempfile::TempDir` for the throwaway `~/.taskfleet/` root, exposed via `TASKFLEET_ROOT_DIR` env (or whatever the binary respects).
  - A throwaway tmux session name (`taskfleet-e2e-<pid>`) so the test doesn't touch the developer's main session.
  - A throwaway workmux config that targets the test session.
  - On Drop: kill tmux session, remove temp dirs, kill any straggler PIDs (reuse the `TestHome` reaper added in the prior fix).
- The spinoff's prompt is a self-contained "touch a marker, commit it, merge yourself, submit node.report success" brief — short enough that a stub agent (or even a shell script masquerading as `claude`) can execute it in seconds.
- Optional stub: a fake `claude` binary on PATH that reads the prompt file and executes the steps directly, so the test doesn't need a real LLM API. This is the harder part — needs design.

Alternative: rather than stubbing the agent, the harness could call a fixed helper script (`scripts/e2e-spinoff-worker.sh`) that does the merge + node.report sequence, bypassing the agent entirely. That tests the taskfleet + supervisor + cleanup chain without the agent-side LLM dependency.

Verification gates the harness enables:

- Watchdog mis-fire regression (the `supervisor-watchdog-misfire-on-fresh-spawn` issue's fix can have an automated test here).
- Headless flag (`taskfleet-headless-spawn` issue's verification).
- Auto-cleanup on failure (does cleanup still fire when the worker reports `success: false`?).
- Multi-spinoff concurrency (spawn N=5 in parallel, all clean up, no orphans).
- Reattach after supervisor crash.

Scope:

- v1: single-spinoff happy-path harness + the cleanup assertions. Feature-flagged.
- v2: add concurrency + reattach + failure-path scenarios.
- v3: extend to /fan-out and /orchestrate end-to-end.

Severity: high for confidence — every future supervisor / cleanup / lifecycle change wants a quick green light here before merging. Without this, every such change requires the developer to hand-craft a live smoke and trust visual confirmation, which is what the previous spinoff had to do.

Related:
- `supervisor-watchdog-misfire-on-fresh-spawn` — uses this harness for regression
- `taskfleet-headless-spawn` — uses this harness for verification
- `workmux-extract-libs` — if/when raine splits workmux, the harness can use the multiplexer crate directly and stub it cleanly
