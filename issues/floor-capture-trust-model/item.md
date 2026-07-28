---
created: 2026-07-22
updated: 2026-07-28
type: task
status: in-progress
priority: high
epic: code-pipeline
related: ['@deterministic-floor', '@plan-check-run-contract']
commits:
- hash: 66eb5332f0322bbbc42e30eb97c8e0cb98a0c6b9
  summary: injection-resistant provenance-bound floor capture
---

# Floor capture trust-model: injection-resistant, provenance-bound evidence for the deterministic floor

## Description

Filed from T3 (`deterministic-floor`). The floor *gates* are pure and
deterministic, but their **inputs are captured by parsing uncontrolled process
text** from a toolchain that runs inside a repository the agent-under-review
controls. design.md §4 asserts the floor is a "mechanical, injection-resistant"
oracle but under-specifies the execution/trust environment that would make that
true. A multi-model review of the T3 diff (Gemini, GPT-5.6, Opus, DeepSeek)
converged on this as the central gap. T3 raised the bar against casual gaming
and landed clear docs about the limitation; this issue tracks closing it before
T5 wires the floor into a live merge gate.

## The gap (why text capture is not yet an oracle)

The agent writes the code/tests/comments AND controls the process environment,
so it can **forge** or **suppress** what the parser sees:

- **libtest output injection** — any line `test <name> ... ok` on stdout is
  trusted. A `println!`, a `build.rs`, or a proc-macro can fabricate passing
  tests (mask a regression, fake a "still present" baseline test). Parsing
  combined stdout+stderr makes provenance meaningless.
- **clippy output injection** — any `… warning: …` line is trusted; a print can
  fabricate a `NewClippyWarning` (DoS the merge) or the summary-filter can drop
  a real one.
- **lint/harness suppression** — `#![allow(clippy::all)]`, `.cargo/config.toml`
  aliases, `rust-toolchain.toml`, feature/target/workspace-member selection, and
  `RUSTFLAGS`/env inheritance all change the observed warning/test set from
  inside the diff.
- **fail-open parsing** — unrecognized lines are skipped, and capture ignores
  exit status, so a compile failure or a `cargo test --bad-flag` yields an
  empty/partial snapshot that passes gates vacuously.
- **test identity is not target-qualified** — `BTreeSet<String>` of bare
  libtest names collides across unit/integration/doctest/package targets, so a
  deleted test can be "replaced" by a same-named no-op in another target.
- **assertion count is not semantic** — even after T3's comment/string
  stripping, `assert!(true)` padding and per-file (not per-test) counting remain
  gameable; whitespace/comment before `!` is missed.
- **baseline is a mutable ref string** — `BaselineSnapshot.ref` is not pinned to
  a commit OID, and the assertion-count maps are passed separately from the
  snapshot with nothing proving they were captured at the same commit by the
  same algorithm.

## What T3 already did (interim mitigations, landed)

- `count_assert_macros` strips comments + string/char literals before counting
  (kills comment/string padding).
- clippy warning identity strips the `:line:col` span (a line-shifting refactor
  no longer flips an unchanged warning to "new").
- newly-ignored gate catches was-failing→ignored, not just was-passing→ignored.
- `hash_sorted` is length-prefixed (unambiguous framing).
- git helpers run with `LC_ALL=C` + `core.quotePath=false`; `file_at_ref` decides
  existence by exit code (`rev-parse` + `cat-file -e`), not English stderr.
- file-scope de-dups changed paths; `Violation`/`GateKind` are `#[non_exhaustive]`.
- module docs state the trust-model limitation plainly and point here.

## What this issue must deliver (before T5 live wiring)

1. **Structured capture, not text**: `cargo test`/`clippy` via
   `--message-format=json` (or `cargo-nextest` for tests), binding each
   observation to its package/target/executable; reject on unparseable/partial.
2. **Fail-closed captures**: surface exit status + per-harness announced-vs-parsed
   counts; a baseline capture must prove complete compilation + execution.
3. **Target-qualified `TestId`** replacing bare-string identity.
4. **Execution isolation**: `env_clear()` + explicit allowlist, wall-clock
   timeouts, stdout/stderr byte caps, process-group termination (overlaps
   design §9 circuit-breakers and issue `outright-tasty-son`); pin toolchain +
   run checks from a trusted config, not a repo-controlled shell alias.
5. **Baseline provenance**: resolve `ref`→commit OID at capture; make the
   baseline artifact atomically carry snapshot + assertion counts + declared
   scope + command/toolchain fingerprints + schema version; add a
   `verify_plan_baseline(artifact, plan::Baseline)` the evaluator requires.
6. **Semantic assertion / coverage signal**: count assertions inside `#[test]`
   items (AST), and decide whether coverage becomes a real gate.
7. **Scope policy** beyond numeric slack: a trusted allowlist +
   `forbidden-even-if-declared` set for validation-control files
   (`.cargo/config*`, `rust-toolchain.toml`, build scripts, CI).

## Notes

- The `plan::Check::run` shell-string-vs-`{cmd,cwd,expect_exit}` contract
  (`plan-check-run-contract`) is a prerequisite for the isolation work.
- This is design work T5 owns; the floor module must not be described as
  tamper-proof until it lands.

