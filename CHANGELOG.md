# Changelog

All notable changes to `orchestratectl` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial CHANGELOG.

## [0.1.0] — pre-release

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

### Fixed (highlights from the MVP + follow-up campaigns)

- Append + projection are persisted under one flock; lock is held until
  every projection file is fsynced.
- Supervisor watchdog no longer false-fires during fresh agent spawns.
- Terminal cleanup completes the run AND removes the worktree, tmux
  window, and branch in one supervisor pass on `node.report`.
- `orchestrator.decision` and `discuss.critical` event kinds are accepted
  by the validator.

### Known gaps (gating v0.1.0 publish)

The polish-bug campaign tracked in `TODO.md` is in flight. Open at
release-cut time:

- `/orchestrate` smoke surfaced 4 polish bugs (headless parent session,
  orchestrated source branch, phantom child on failed spawn,
  supervisor cleanup `--force`).
- 4 data-integrity bugs in the reducer / event-log durability path
  (`applied_seq` watermark, torn-write truncation, `recover_last_seq`
  empty-line loop, manifest counter desync).
- A handful of read-side / API / output cleanups tracked under Phase C
  of `TODO.md`.

Zero open issues is a release gate. See `TODO.md` for the active
sequence.

[Unreleased]: https://github.com/jarimustonen/orchestratectl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jarimustonen/orchestratectl/releases/tag/v0.1.0
