---
created: 2026-07-22
updated: 2026-07-22
type: task
status: fixed
priority: high
commits:
- hash: 7d765dc
  summary: CodeHarness trait + aider adapter + conformance suite (T0)
- hash: 24b487f
  summary: address llm-review findings (base verify, dirty/ancestry outcome mapping, check-id conformance, Send+Sync)
closed: 2026-07-22
---

# CodeHarness adapter interface + ChunkRequest/ChunkResult protocol + conformance suite (aider first adapter)

## Description

Foundational seam of the code-pipeline epic (`issues/code-pipeline`, breakdown
T0; design §10, §5). Define a versioned, harness-neutral `CodeHarness` contract
so the supervisor can drive a code-writing agent (any model/tool) over one chunk
and consume a **structured** `ChunkResult` — never tool-specific prose, never
exit-status guessing. Ship `aider` as the first conforming adapter. Everything
downstream (T1 tier binding, T4 control loop, T5 staged supervisor) binds to this
seam.

Behavior-preserving scaffolding: lands as a new, unused-by-default module +
tests. NOT wired into the live `run create` / supervisor spawn path — staged
rollout (design §14) plugs it in later.

## Scope delivered

`crates/taskfleet-cli/src/harness/`:

- **`mod.rs`** — the `CodeHarness` trait (`capabilities()` + `run_chunk()`), the
  serde-serializable protocol types (`ChunkRequest`, `ChunkResult`,
  `ChunkOutcome`, `Check`, `CheckResult`, `Usage`, `HarnessCapabilities`), and a
  structured `HarnessError` (provider failure / malformed output / dirty worktree
  / invalid worktree / missing credential / internal). `HARNESS_CONTRACT_VERSION`
  stamps every result for provenance.
- **`aider.rs`** — `AiderHarness`: shells out non-interactively with the
  spike-proven invocation, commits but does NOT merge, reads the outcome from the
  resulting git commit (synthesizes `NoChange` when no commit is produced), runs
  the request's checks as the self-check, and best-effort parses usage. Model id +
  credential env-var name are config; the key is read from the environment
  (`DEEPSEEK_API_KEY` by default), never hardcoded. `GIT_BIN` / `TASKFLEET_AIDER_BIN`
  overrides make it testable with no network.
- **`stub.rs`** — `StubHarness`: a deterministic in-process fake the conformance
  suite drives in CI (no network, no git).
- **`conformance.rs`** — `assert_result_conforms` (adapter-agnostic structural
  invariants) + `run_and_check`, plus the design §10 scenario matrix (clean
  success, no-change, self-check failure, failed/malformed run, dirty worktree,
  timeout/cancel, transcript+usage capture) run against the stub by default; the
  aider adapter is exercised through the same contract gate with a fake `aider`
  binary.

## Comments

- The interface issue enumerated `ChunkRequest` as "at least" its listed fields;
  a `files` field (declared file scope, design §4 `files_touched[]`) was added
  because a file-oriented adapter (aider) needs the edit scope. No other
  undocumented behavior was invented.
- `Timeout`/`Cancelled` live in `ChunkOutcome` (a *completed* run's verdict), not
  in `HarnessError` (which is reserved for "could not produce a result at all").

