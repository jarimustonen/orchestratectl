---
created: 2026-07-28
updated: 2026-07-31
type: improvement
status: in-progress
priority: high
epic: code-pipeline
related: ['@floor-capture-hardening-round-2']
blocked_by: ['@floor-capture-hardening-round-2']
---

# Floor capture hardening round 3: structured cargo invocation, expected-target manifest, doctests/custom-harness, provenance enforcement wiring

## Description

Follow-up to `floor-capture-hardening-round-2`. Round 2 landed the highest-leverage
mechanical fixes (fresh per-capture `CARGO_TARGET_DIR` for F4, the enumeration-superset
gate for F7, and cross-component provenance in `verify_plan_baseline` with a
fail-closed-on-empty-provenance guard). A `/llm-review` panel (Gemini 3.1 Pro, GPT-5.6,
Opus 4.7, DeepSeek v4; triage in `history/assessment-floor-capture-hardening-round-2.md`)
confirmed those but surfaced the structural limits that remain. They cluster into one
theme: **the floor still composes a repo-influenced `sh -c` cargo command and judges
coverage only relative to the fork baseline**, so an adversarial repo can still steer or
starve the capture.

## Deliver

1. **Structured cargo invocation (root-cause completion).** Stop running floor-owned
   test/clippy captures as `sh -c "<string>"`. Invoke a supervisor-resolved cargo via
   argv (`Command::arg`), against a sanitized environment/config: neutralize repo
   `[alias]` (esp. a `clippy` redirect), `build.rustc` / `build.rustc-wrapper` (a
   compiler wrapper that suppresses diagnostics), config `rustflags` / lint-level flips,
   and `[env]` overrides — via `--config` locks or a supervisor-generated CARGO_HOME/
   out-of-tree invocation. Removes the `inject_cargo_flags` whitelist bit-rot and the
   whitespace/quoting fragility entirely.

2. **Independent expected-target manifest (F7 completeness).** Derive the expected
   `(package, target_kind, target)` set from trusted `cargo metadata` and require the
   captured enumeration to match it — so a *compromised or already-empty* baseline is
   caught, not just a narrowing relative to a (trusted) non-empty baseline. Add a
   parallel clippy target/coverage fingerprint and a feature-set fingerprint so tests
   and clippy are proven to cover the same source/target universe at baseline and tip.
   Consider failing closed on an empty enumeration when metadata says test targets exist.

3. **F5 — custom harness forge.** Reject an undeclared `harness = false` on a
   *test-producing* target (`[[test]]` / `[lib]` / `[[bin]]`), while allowing a
   legitimate `[[bench]] harness = false` (this repo has one — criterion-style benches).
   Needs Cargo.toml parsing that distinguishes target kinds; a blanket reject is wrong.

4. **F6 — doctest capture.** Doctests run via rustdoc, not a `compiler-artifact` binary,
   so a new failing doctest (or a test moved *into* a doctest) is invisible. Add a
   `cargo test --doc` pass (structured/text-parsed + reconciled, `target_kind="doctest"`)
   or the nightly libtest JSON format; or fail closed if doctests exist but are
   uncaptured. Symmetric across baseline/tip → a gaming hole, not a crash.

5. **Provenance robustness (trigger: wiring `verify_plan_baseline` into the T5
   evaluator).** `verify_plan_baseline` is groundwork today (no caller). When wiring it:
   validate `commit_oid` as a full-length git OID; make the toolchain check
   semver-tolerant (an exact `rustc -V` string false-blocks on a patch/nightly-date bump);
   verify `HEAD == commit_oid` and a clean worktree *before* capture (a matching OID does
   not prove the captured files were that OID); reject `toolchain == "unknown"`; and bump
   the plan schema so provenance fields become *required* in the enforcing version rather
   than silently `#[serde(default)]`.

6. **Capture resource ceilings** (fresh dir per capture = full recompile × baseline/each
   chunk/feature): wall-clock timeout, target-dir size cap, tmpfs-full / free-space
   check, process-group termination, cleanup-on-panic. Coordinate with the §9
   circuit-breakers issue `outright-tasty-son` (may live there, not here).

## Out of scope / accepted (unchanged from round 2)

- Semantic per-`#[test]` AST assertion counting (still a crude per-file `assert*!` count).
- `forbidden-even-if-declared` file-scope policy for validation-control files.
- Structured `TestTargetId` key instead of the `/`-joined string — cleanliness only;
  cargo forbids `/` in package/target names so there is no current collision risk.
- Hard-blocking legitimate target removal/rename via the superset gate is intended
  (a removed test target is a real coverage drop); an authorized-override escape hatch
  is future work, folded into item 2.

Blocked-by: `floor-capture-hardening-round-2` (this builds on its snapshot/plan shape).
