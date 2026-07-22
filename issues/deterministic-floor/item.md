---
created: 2026-07-22
updated: 2026-07-22
type: task
status: in-progress
priority: high
---

# Deterministic floor: baseline snapshot + supervisor-enforced gates (tests/clippy vs baseline, file-scope, test-gaming) as standalone module

## Description

The mechanical correctness FLOOR the code-pipeline needs below LLM verify
(design.md §4 — the panel's #1 non-negotiable). Built as a standalone,
fully-tested module of pure functions in `crates/octl-cli/src/floor/`, behind
the seam (design §14): **not** wired into `run merge`/the supervisor yet —
behavior-preserving scaffolding + tests that T5 will plug into the merge gate.

## What landed (`crates/octl-cli/src/floor/`)

- **`snapshot.rs`** — the pure value model: `TestSnapshot` (passed/failed/ignored
  id sets), `ClippySnapshot`, optional `Coverage`, `RunSnapshot`, and
  `BaselineSnapshot` (pinned to the fork ref) with `sha256` hashing that projects
  down to the `octl_core::plan::Baseline` hash-only shape. Plus `CheckRun`.
- **`parse.rs`** — pure parsers (no I/O): libtest human output → `TestSnapshot`,
  clippy `--message-format=short` → `ClippySnapshot`, and a crude `assert*!`
  counter for assertion-density. Exhaustively fixture-tested.
- **`gates.rs`** — the five pure gates + `evaluate_floor` → structured
  `FloorVerdict`/`GateOutcome`/`Violation`: checks-pass, no-regression,
  no-new-clippy, no-test-gaming (count drop / newly-ignored / vanished-test /
  assertion-density regression), file-scope (`files_touched[]` + configurable
  slack).
- **`runner.rs` + `git.rs`** — the thin *impure* capture layer (run
  checks/tests/clippy via `sh -c`; count assertions on disk / at a git ref via
  `git show`; `git diff --name-only` for changed files). Keeps the gates pure.

Reuses `octl_core::plan::{Check, Baseline}` (does not redefine them). Makes no
LLM calls and no judgments — deterministic set/inequality rules only. Touches no
event-append/reducer/lock path (state-integrity invariants).

## Verification

- 45 floor unit tests (fixtures + temp git repos, no network); `cargo test
  --workspace` green.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets` no new
  warnings.
- CHANGELOG "Unreleased" entry added.

## Follow-ups (not blocking)

- Check-run contract (`plan-check-run-contract`): the runner executes
  `plan::Check::run` as a shell string via `sh -c`; the richer
  `{cmd,cwd,expect_exit}` contract is a separate open decision. Seam left in
  `runner::run_check`.
- T5 wires `evaluate_floor` into the supervisor's chunk-/feature-merge gate.

