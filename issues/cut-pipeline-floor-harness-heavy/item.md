---
created: 2026-08-14
updated: 2026-08-14
type: task
status: done
priority: high
epic: lifecycle-architecture-review
labels: [architecture]
closed: 2026-08-14
closed_by: agent-cut
---

# 0.2 subtractive cut: remove pipeline/floor + harness heavy layer

## Description

First subtractive cut of the 0.2 simplification (ADR `docs/decisions/0001-thin-supervisor-vs-harden.md`, Migration sketch step 1; DECISION-1 `target-state-0.2.md`). Largest, most bisectable deletion — land it behind a green integrated gate before any of the thin-supervisor rework.

**Scope (this cut only):**
- Delete the code-pipeline subsystem: `crates/octl-cli/src/pipeline/*` and `crates/octl-cli/src/floor/*`, and any CLI surface / commands that only exist to drive them.
- Delete the harness **heavy layer**: `harness bakeoff`, `harness conformance`, the `CodeHarness` trait machinery, and the `aider` + `claude-deepseek` adapters. **Keep the light claude + pi launcher** (`--harness claude|pi` must still work — this is the 0.2 harness surface).
- Remove now-dead wiring, tests, docs, and bundled-registry references for the above.

**Out of scope (later, sequenced cuts — do NOT do here):** removing run *kinds* (`code`/`orchestrate`/`orchestrated`/`bugfix`/`make-skill`), the discussion/spinoff-proposal machinery, or any `supervise/*` lifecycle-inference rework. Those touch `supervise/*` + the skill bundle and are separate units.

**Constraints:**
- Do NOT touch `crates/octl-cli/src/skill.rs` or the pi-provenance path — a parallel worktree owns it this round.
- `{harness,floor,pipeline}/*` are on the correctness-sensitive never-parallelize list; this is the sole owner of that subtree this round.
- Keep `octl-core/{events,lock,reducer,schema}.rs` and `supervise/*` behavioural changes to the minimum needed to unwire the deleted modules (ideally none).

**Acceptance:** green gate (`cargo fmt --all`, `cargo clippy --workspace --all-targets` no new warnings, `cargo test --workspace`) + the insta snapshot loop for any changed CLI surface; `orchestratectl --harness pi` and `--harness claude` still launch. Close the now-obsoleted pipeline/harness fix issues (`pipeline-hardening`, `pipeline-run-create-wiring`, `pipeline-breaker-inflight-and-opus-metering`, `pipeline-drop-primitive-underspecified`, `pipeline-tiered-triage`, `dreadfully-dirty-pain`, `practically-exclusive-celery`) as `obsolete` (superseded by this cut). Production code → run `/llm-review` (+ `/assess-findings`) before merging.

## Resolution

### 2026-08-14T04:43:09Z · @agent-cut

0.2 subtractive cut landed: deleted pipeline/* + floor/* + harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek/stub, ~26.5k LOC), kept light claude+pi launcher (--harness claude|pi still launches). proc.rs run_with_control/ControlledOutcome/StopReason (harness-only) inlined into run_with_timeout, behavior-preserving (4-model /llm-review consensus). Green gate + integrated green + insta help snapshots regenerated (version_* untouched). Obsoleted 7 pipeline/harness issues.
