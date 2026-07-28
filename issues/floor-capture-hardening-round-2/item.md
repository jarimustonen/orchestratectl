---
created: 2026-07-28
updated: 2026-07-28
type: improvement
status: in-progress
priority: high
epic: code-pipeline
related: ['@floor-capture-trust-model']
commits:
- hash: d36457b
  summary: round-2 floor hardening (F4 target-dir, F7 enumeration, provenance)
---

# Floor capture hardening round 2: repo-controlled cargo config, enumeration integrity, doctests, provenance enforcement

## Description

# Floor capture hardening round 2 — repo-controlled cargo config, enumeration integrity, doctests, provenance enforcement

Follow-up to `floor-capture-trust-model`. That issue delivered the injection-resistance
core (structured JSON capture, fail-closed, target-qualified `TestId`, env isolation,
OID-pinned baseline). A multi-model review (Gemini 3.1 Pro, Claude Opus 4.7, DeepSeek v4;
triage in `history/assessment-floor-capture-trust-model.{json,md}`) confirmed the cheap
fixes (exit-status, `filtered out`, flag-injection-before-`--`, LD/DYLD allowlist,
`CARGO_INCREMENTAL=0`) — all landed — and surfaced five residual bypasses that need their
own design. They share one root cause: **cargo still reads repo-controlled configuration
and trusts the workspace layout**, so `env_clear()` alone cannot neutralize them.

## What this issue must deliver

1. **Neutralize in-repo cargo config / aliases / toolchain overrides** (root cause).
   Run capture with `--config` overrides or from out-of-tree so a committed
   `.cargo/config.toml` (`[alias]`, `build.target-dir`, `[env]`, lint levels),
   `rust-toolchain.toml`, or workspace-narrowing cannot change the observed set.
   - **F4 — clippy cache bypass:** an in-repo `build.target-dir` can point baseline and
     tip at one shared cache; `cargo clippy` on a warm cache re-emits **zero** warnings,
     so `gate_no_new_clippy` passes vacuously. Pin a floor-controlled `CARGO_TARGET_DIR`
     per capture (+ `--config` override that beats in-repo config). `CARGO_INCREMENTAL=0`
     is already set but does not cover a shared target dir.

2. **Enumeration integrity (F7).** A `build-finished:true` run with zero (or narrowed)
   test binaries captures an empty snapshot that passes vacuously when the narrowing
   predates the fork. Fingerprint the enumerated `(package, target_kind, target)` set into
   the baseline artifact and require the tip's set to be a **superset**; fail closed on a
   shrink.

3. **Custom harness (F5).** `harness = false` lets a hand-written `main()` print perfectly
   balanced forged libtest output that the announced-vs-parsed reconcile cannot detect on
   stable. Lock down `Cargo.toml` (reject undeclared `harness = false`) and/or pass a
   libtest-only flag a custom harness would reject.

4. **Doctest capture (F6).** Doctests run via rustdoc, not a `compiler-artifact` binary,
   so they are dropped from both baseline and tip — a new failing doctest, or a test moved
   *into* a doctest, is invisible. Add a `cargo test --doc` pass (text-parsed + reconciled,
   `target_kind = "doctest"`), or adopt the nightly libtest JSON format. NOTE: capture is
   symmetric across baseline/tip, so this does NOT break the pipeline "by default"
   (DeepSeek's claim to that effect was assessed INCORRECT); it is a gaming hole, not a
   crash.

5. **Provenance enforcement in the evaluator (F10).** `BaselineSnapshot::verify_plan_baseline`
   currently compares the mutable `ref` string, not the pinned `commit_oid`, and does not
   compare the toolchain fingerprint. Extend `plan::Baseline` (octl-core) to carry
   `commit_oid` + toolchain and have the T5 evaluator require OID + toolchain equality, not
   just the two hashes.

## Open decision (F13 — DISCUSS)

Clippy warning identity is currently `(lint, package, file, message)`. Including `message`
makes a legitimate rename (`unused variable: \`x\`` → `\`y\``) look like a new warning +
a vanished one, spuriously firing `NewClippyWarning`. Decide the identity granularity:
`(lint, package, file)` (coarser — stops distinguishing two same-lint instances in one
file, related to the accepted F12 collapse trade-off) vs. keeping `message` as identity.
This is a granularity trade-off, not a clear defect.

## Out of scope (tracked elsewhere / accepted)

- Semantic per-`#[test]` AST assertion counting (still a per-file crude `assert*!` count).
- `forbidden-even-if-declared` file-scope policy for validation-control files.
- Capture wall-clock timeouts / output caps / process-group termination — belong with the
  §9 circuit-breakers (`outright-tasty-son`).
- F12 (two identical warnings collapse behind `#[allow]`): accepted narrow trade-off
  (DROP 1c); revisit here only if it recurs.
