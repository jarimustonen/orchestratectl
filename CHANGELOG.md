# Changelog

All notable changes to `orchestratectl` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Supervisor reconciles run status with git after a self-merge.** A spinoff
  whose branch already merged into its `source_branch` is no longer reported
  `failed` (issue `false-failed-after-merge`) or left stuck at `pending`
  forever (issue `supervisor-stuck-pending-after-self-merge`) when its terminal
  `node.report` is lost or never flushed under high fan-out. The watchdog now
  checks `git merge-base --is-ancestor <branch> <source>` (plus an
  advanced-past-fork-point guard using the new `Node.base_sha`) before any
  terminal classification: a merged branch synthesizes a terminal SUCCESS
  (`via: "merge-reconciled"`) and tears down cleanly — even while the agent is
  still alive — instead of a false `agent-died`/`cleanup.branch_preserved`.
  Reconciliation is deliberately conservative so it can never destroy live work:
  it requires the branch to have advanced *forward* past its fork point
  (`base..branch > 0`, rejecting a fresh or `reset`-rewound branch) AND the
  worktree to be clean (an agent that merged then kept editing keeps its
  uncommitted work), re-verifies the merge under the run lock before recording
  it, and re-checks the source-relative unmerged gate at teardown so a branch
  that diverged after the report is preserved, not force-deleted.

## [0.1.0] — 2026-07-04

First publishable cut. The CLI is real, the bundled skill family covers
the full agent loop, and run state survives crashes via an append-only
event log + lock-gated reducer.

### Added

- **Run model.** Every spawn is a `run` (`~/.orchestratectl/runs/<ulid>/`)
  with `events.jsonl` as the canonical source of truth and
  `manifest.json` / `nodes/` / `discussions/` / `spinoffs/` as
  projections reduced under a single per-run flock.
- **Run create kinds.** `code`, `spinoff`, `orchestrated`, `research`,
  `bugfix`, `technical-decision`, `make-skill`, `fan-out`, `orchestrate`.
- **Run merge.** `orchestratectl run merge <run-id> [--report-file]`
  rebases + merges the worktree branch and submits the terminal node
  report in one call; supervisor tears down worktree + tmux window +
  branch automatically.
- **Supervisor.** Per-run watcher with a fresh-spawn grace window
  (no false watchdog misfires), terminal cleanup on `node.report`,
  detached-PTY support via `--headless` / `--tmux-session`.
- **Skill bundling.** 13 Claude Code skills bundled in the binary and
  deployed via `orchestratectl skill install --force`:
  `orchestratectl-overview`, `octl-run-overview`, `octl-spawn-spinoff`,
  `worktree-code`, `worktree-spinoff`, `worktree-merge`,
  `worktree-research`, `worktree-bugfix`, `worktree-technical-decision`,
  `worktree-make-skill`, `worktree-orchestrated`, `fan-out`,
  `orchestrate`. SKILL examples are CI-gated against the actual binary
  CLI surface.
- **Doctor.** `orchestratectl doctor` reports schema, install, and
  skill-sync health (current: 63 ok / 0 fail).
- **AI-first CLI.** Every command follows the conventions in
  `AGENTS-AI-FIRST-CLI.md` (`--json` everywhere, JSONL logs, strict
  input validation, informative error envelopes, no interactive prompts).
- **`run create --agent-startup-timeout <seconds>`** (1–600, default 90).
  Forwarded to `create.sh`; higher than create.sh's own 30s default
  because octl batch-spawns self-load the host and a fresh agent can miss
  a 30s window under load. Closes `run-create-agent-startup-timeout`.
- **Supervisor liveness surface.** `run show` / `run list` report
  `supervisor: {pid, alive}` (orphan detection), and `run merge` returns a
  machine-readable `supervisor: {state}` outcome
  (`alive | terminal | not-supervised | reattached | deferred`).

### Fixed (highlights from the MVP + follow-up campaigns)

- Append + projection are persisted under one flock; lock is held until
  every projection file is fsynced.
- **append + apply atomicity via `applied_seq` watermark** (`361839f`):
  the writer advances a per-manifest `applied_seq` only after every
  projection an event touches is fsynced; on next lock acquisition
  unapplied tail events (`seq > applied_seq`) are replayed before any
  new append, and an idempotency-key replay catches the projection up
  before returning. Legacy manifests self-migrate.
- **`events.jsonl` torn-write recovery** (`395ba03`): the recovery
  path truncates a partial trailing line at the byte boundary after
  the last fully-parseable event and warns about the discarded bytes.
- **`recover_last_seq` whitespace-tolerant** (`cc4ff46`): skips
  trailing whitespace-only lines instead of choking.
- **`run create --headless` no longer crashes create.sh** (`5ce764d`):
  Rust-side regression tests pin the `--parent-session` forwarding
  contract; the homebase-side fix landed alongside.
- **Orchestrated child worktrees fork from `--source-branch`**
  (`145905f`): not from `main`, so `/orchestrate` DAG dependencies hold.
- **Failed `create.sh` no longer leaves a phantom child** (`438aa29`):
  `child.spawned` is emitted only after create.sh returns success.
- **Supervisor cleanup `git worktree remove --force`** (`11a5850`):
  disposable untracked scratch in a worktree no longer orphans the
  worktree+branch.
- **`worktree-merge` recovers a renamed window** (`bfd7bfb`): the
  supervisor's cleanup falls back to a worktree-path lookup when the
  tmux window has been renamed or detached.
- Supervisor watchdog no longer false-fires during fresh agent spawns.
- Terminal cleanup completes the run AND removes the worktree, tmux
  window, and branch in one supervisor pass on `node.report`.
- `orchestrator.decision` and `discuss.critical` event kinds are accepted
  by the validator.
- **`run merge` no longer reports silent success when the supervisor is
  dead** (`supervisor-dead-merge-no-teardown`): it reads status +
  supervisor-pid liveness + the event log as one shared-locked decision
  and auto-reattaches a fresh supervisor when the recorded one is dead and
  the run is non-terminal, so the terminal report is consumed and teardown
  completes. Never silent — emits a warning plus the `supervisor: {state}`
  outcome above.
- **Blocked terminal report preserves the branch + worktree**
  (`blocked-report-deletes-branch`, high-severity silent data loss): a
  `node report` with `success: false` (the needs-a-human path) no longer
  force-deletes the worktree branch. Teardown is gated on the terminal
  outcome (`node_report_is_blocked`) plus a source-relative
  `git rev-list --count <source>..<branch>` safety net; `git branch -D`
  force-delete is reserved for a confirmed `run merge`
  (`via: "explicit-merge"`).
- `supervise_gates` + `e2e_spinoff` test binaries serialize on a process-
  wide file lock (`/tmp/octl-test-supervise.lock` via `serial_test`'s
  `#[file_serial]`), removing a `cargo test --workspace` self-terminate
  flake. Closes `flaky-self-terminate-test`.

### Known gaps (carried to v0.2)

- `runwriter-batched-append-api` — a long-lived `RunWriter` guard with
  cached `next_seq` + batched fsync to cut the V4 append p99 (639ms) to
  the 10ms budget. Overlaps the just-landed `applied_seq` watermark /
  `LockedRun` witness / `AppendOutcome` primitives, so it lands cleaner
  once those have shaken out. Append p99 is well within budget for the
  one-per-action write cadence today; tight back-pressure loops (a
  supervisor batching many events) will surface it.
- `cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`,
  `teardown-gate-trust-and-lifecycle` — supervisor-lifecycle follow-ups
  filed during the pre-1.0 hardening pass; none is a data-loss or
  correctness blocker (the blocked-report source-relative net already
  covers committed-work preservation on the cancel path).

[Unreleased]: https://github.com/jarimustonen/orchestratectl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jarimustonen/orchestratectl/releases/tag/v0.1.0
