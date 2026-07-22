---
created: 2026-07-22
updated: 2026-07-22
type: task
status: fixed
priority: high
closed: 2026-07-22
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

## Review

`/llm-review` (Gemini 3.1 Pro, GPT-5.6-sol, Opus 4.7, DeepSeek v4 Pro) run on
the final diff. In-scope pure-layer findings fixed with tests (clippy-identity
span-strip, comment/string stripping in the assertion counter, was-failing→
ignored detection, length-prefixed hash, exit-code-based git existence + LC_ALL=C,
stdout/stderr newline join, file-scope de-dup, `#[non_exhaustive]` on serialized
enums). The consensus critical finding — the capture layer parses uncontrolled
process text and is steerable by the agent-under-review — is an
execution-environment decision T5 owns; filed as `floor-capture-trust-model` and
documented as a limitation in the module. Report: `history/review-deterministic-floor.md`.

## Follow-ups (not blocking)

- **`floor-capture-trust-model`** (filed): injection-resistant, provenance-bound
  capture (structured JSON, fail-closed, target-qualified test ids, execution
  isolation, baseline ref→OID) — prerequisite for T5 live wiring.
- Check-run contract (`plan-check-run-contract`): the runner executes
  `plan::Check::run` as a shell string via `sh -c`; the richer
  `{cmd,cwd,expect_exit}` contract is a separate open decision. Seam left in
  `runner::run_check`.
- T5 wires `evaluate_floor` into the supervisor's chunk-/feature-merge gate.

