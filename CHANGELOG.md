# Changelog

All notable changes to `orchestratectl` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`run create --notify <cmd>` completion hook (`no-completion-notification-to-parent`).**
  A run created with `--notify <cmd>` now runs that command **exactly once** when
  the run reaches a terminal state (`done | failed | cancelled`), fired by the
  supervisor on the terminal transition **before** teardown removes the
  worktree/window. The command runs via `sh -c` with `OCTL_RUN_ID`, `OCTL_STATUS`,
  `OCTL_SUMMARY`, `OCTL_RUN_KIND`, and `OCTL_RUN_TITLE` in its environment — the
  push signal a spawning session needs to learn of completion without polling
  (append a line to a file the harness watches, post a desktop toast, ping a
  FIFO). At-most-once is durable: firing is gated on a `run.notified` marker event
  (idempotency key `supervisor-notify:<run-id>`) appended and fsynced before the
  spawn, so a supervisor restart replays it as a no-op and never re-fires. The
  command is spawned detached so a slow/hung hook cannot wedge the supervisor
  tick. New manifest field `notify_cmd` (`#[serde(default)]`, absent when the flag
  is omitted). The bundled `worktree-spinoff` and `worktree-code` skills document
  `--notify` and backgrounded `run wait` as the two ways completion reaches the
  spawning session, and no longer imply an undeliverable notification;
  `worktree-code`'s progress section is also corrected to branch on
  `manifest.status` (not `lifecycle`).

- **Flexible `plan.json` check contract (code-pipeline `plan-check-run-contract`,
  owner-locked 2026-07-23).** A check now carries the general goal (`desc`) plus a
  flexible shell command (`run`) with optional precision — `cwd` (repo-relative
  working directory) and `expect_exit` (expected exit code, default 0) — on both
  per-chunk `checks[]` and `acceptance[]` check items. Neither a rigid struct nor
  bare text: the goal is always communicated, the command stays expressive, and
  precision is available but not forced. `cwd` is held to the same repo-relative
  safety guard as `files_touched` (the floor gates possibly-adversarial code-node
  output, so an absolute / `..` / `~` cwd is rejected), and `expect_exit` is
  bounded to the shell exit range `0..=255`. The `plan::Check` type, structural
  validator, checked-in JSON Schema, and the deterministic-floor runner (which
  honours `cwd`/`expect_exit` and records `cwd` on the `CheckRun` audit result)
  are updated in lockstep; a check with only `desc`+`run` is unchanged
  (exit 0 = pass).

- **Inverted control loop scaffold (code-pipeline T4, behind the seam).** A new
  module (`crates/octl-cli/src/pipeline/`) modelling the design §2 inversion: the
  supervisor owns the loop and the orchestrator is a stateless pure function
  returning discrete typed `Action` primitives (`ReCodeChunk`, `TriggerReSpec`,
  `AcceptChunk`, `PromoteTier`, `OpenDiscussion`, `ProposeSpinoff`,
  `DeclareConverged`, `Escalate`) — never prose. Each primitive is classified
  `Routine` vs `Consequential` (design §0.2; spin-off triviality carried by an
  explicit `SpinoffScope`, not overloaded onto finding `Severity`), and a
  `TieredOrchestrator<C, D>` routes consequential decisions from a fast
  coordinator to an expensive decider. Every decision is recorded atomically as a
  `DecisionRecord` (action + `DecisionEnvelope` with actor/inputs/reason/
  **`decision_tier`**/model/prompt-version + outcome), so the trail is causally
  replayable. The `drive` loop is fail-closed: a consequential action stamped
  `coordinator` is a caught `TierViolation` that escalates; a circuit-breaker trip
  escalates deterministically without consulting the orchestrator (design §9);
  and chunk preconditions are checked before any would-execute. Ships
  deterministic scripted coordinator/decider stubs and a stubbed `ActionExecutor`,
  fully unit-tested (routine FIX loop, decider-tier `DeclareConverged`/`Escalate`/
  `TriggerReSpec`, mis-tier rejection, superseded post-terminal actions, unknown-
  chunk rejection) with no LLM/network. Reviewed via `/llm-review` (3 models);
  real findings addressed. Unused by default — nothing constructs an
  `Orchestrator` or calls `drive` yet; T5 wires it into the real supervisor +
  event log (design §14).
- **`CodeHarness` execution control: timeout + cancellation (code-pipeline T0
  follow-up, still behind the seam).** `run_chunk` now takes a `CancelToken` and
  `ChunkRequest`/`Check` carry optional wall-clock `timeout`s, so a runaway or
  hung code-node can be bounded and aborted (design §9 circuit-breakers). The
  `AiderHarness` honours both — killing the agent's (and each check's) process
  group on expiry/cancel, draining the partial transcript, and returning
  `ChunkOutcome::Timeout`/`Cancelled` (never a hang); `StubHarness` gained a
  `SlowUntilCancel` behaviour so the conformance suite tests both deterministically
  with no network. Unblocks live wiring (T5).
- **Deterministic correctness floor (code-pipeline T3, behind the seam).** A
  standalone module (`crates/octl-cli/src/floor/`) of pure gate functions plus a
  thin capture layer implementing the mechanical merge floor (design §4): a
  serde `BaselineSnapshot` captured at the `feat/<slug>` fork (test pass-list +
  clippy-warning-list hashes projecting down to `plan::Baseline`, optional
  coverage), a check runner over `octl_core::plan::Check`, and five pure gates —
  checks-pass, no-regression (no baseline-passing test now fails), no-new-clippy,
  no-test-gaming (count/ignore/rename/assertion-density), and file-scope
  (`files_touched[]` + slack) — returning a structured `FloorVerdict` of
  `Violation`s. No LLM calls, no judgment, fully unit-tested from fixtures + temp
  git repos, no network. Unused by default — not wired into any live
  `run merge`/supervisor path; staged rollout (design §14) plugs it into the
  supervisor merge gate at T5.
- **`plan.json` v2 schema + validator (`octl_core::plan`).** Serde types for the
  code-pipeline stage contract (`schema_version`, immutable `plan_rev`,
  `intent_rev`, `feature`, `baseline`, `acceptance[]` checks/assertions, and the
  `chunks[]` DAG) plus a structural validator that rejects unsupported schema
  majors and undeclared fields, enforces unique/acyclic chunk deps, ≥1 executable
  check per chunk and in `acceptance[]`, and safe repo-relative `files_touched`
  paths — with domain-typed `PlanValidationError`s the CLI can map to its
  `schema_violation` envelope. A checked-in Draft 2020-12 JSON Schema
  (`crates/octl-core/schemas/plan.v2.schema.json`) is the machine-readable
  artifact, kept in sync with the Rust types by a drift-guard golden test. Read-
  only types + validation only — not yet wired into a live path (design.md §4/§7/
  §13, `issues/code-pipeline/plan-schema.md`).
- **`CodeHarness` adapter contract (code-pipeline T0, behind the seam).** A
  versioned, harness-neutral trait (`crates/octl-cli/src/harness/`) that lets the
  supervisor drive a code-writing agent over one chunk and consume a structured
  `ChunkResult` — never tool prose or exit-status guessing (design §10). Ships the
  request/result protocol (`ChunkRequest`, `ChunkResult`, `ChunkOutcome`, `Check`,
  `CheckResult`, `Usage`, `HarnessCapabilities`, structured `HarnessError`), an
  `AiderHarness` first adapter (shells out non-interactively, reads the outcome
  from git not stdout, `DEEPSEEK_API_KEY` from the env), a deterministic
  `StubHarness`, and a reusable conformance suite. Unused by default — not wired
  into any live `run create`/supervisor path; staged rollout (design §14) plugs it
  in later.

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
