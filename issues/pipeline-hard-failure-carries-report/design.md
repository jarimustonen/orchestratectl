# Design — carry a PipelineReport on the hard-failure Err path

## Problem

`run_pipeline_tiered` returns `Result<PipelineReport, PipelineError>`. On a hard
`PipelineError` in a concurrent wave, the `?`-propagation drops the local `Run` and
its accumulated `PipelineReport` — including the `branch_preserved` audit entries for
floor-green / blocked siblings that committed work but never merged. Committed sibling
work survives on disk (teardown never deletes chunk branches), but the **audit record
is lost**, so invariant-5 preservation is UNAUDITABLE on the Err path.

Worse: the wave-build hard-error path (`live/mod.rs` ~3068) *deliberately did not even
record* the preservation, precisely because "the Err discards the run's report". So
there was nothing to surface even if we wanted to.

## Decision

Three considered shapes:

1. **`(Report, Result)` tuple return** — every caller must destructure even on Ok;
   noisy, and the report on Ok duplicates the Ok value.
2. **`enum` replacing `PipelineError`** with an `Ok`-carrying variant — conflates the
   typed-error taxonomy (which drives the stable CLI error `code()`) with report
   transport; every `matches!(err, PipelineError::Harness(_))` site breaks.
3. **Wrapper struct on the Err arm** — `PipelineFailure { error: PipelineError, report:
   Option<PipelineReport> }`, return type `Result<PipelineReport, PipelineFailure>`.

**Chosen: option 3.** It keeps `PipelineError` (and its stable `code()`) intact as the
dominant failure signal, is additive (the Ok arm is unchanged), and pairs the report
with the error only where a report is meaningful.

- `report` is `Option` because pre-plan failures (bad repo/branch, integration branch
  already exists, spec failure) have no plan and no accumulated chunk state — a report
  there would be empty and dishonest. `report` is `Some` exactly when the run got far
  enough to accumulate state worth auditing (past spec, into the fix loop / merge).
- `impl From<PipelineError> for PipelineFailure` gives `report: None`, so the
  prologue's `?`-sites (setup + spec, before `plan` exists) keep working unchanged.
- The main body (fix loop + merge, everything after `plan` is built) is wrapped so any
  `PipelineError` it produces is paired with `finalize(&run, &plan, …, "pipeline_error")`
  — a full report carrying every `chunk_reports[*].branch_preserved` entry.
- `impl From<PipelineFailure> for CliError` delegates to the inner `error` (unchanged
  exit-code / envelope mapping — the exit code is NOT downgraded).

## Behavioral change (the actual fix)

On the wave-build hard-error path, we now **preserve the floor-green / blocked siblings
before returning `Err`** (`preserve_wave_build` over `blocked` + `built`), so their
`branch_preserved` records land in `run.chunk_reports`. This is what makes the report
carry the invariant-5 audit. Disk teardown is unchanged (it never touched chunk
branches on this path); this is purely additive audit state.

`cmd_run` renders the carried report (text + `--json`) and THEN returns the error, so a
hard wave failure both **exits non-zero** AND **prints every preserved sibling**.
`print_report` gains a `preserved: <branch>` line per chunk so the audit is visible in
text output.

## Review dispositions (llm-review + source verification)

- The `--json` "double envelope" concern (all 4 reviewers) is **refuted**: the report
  is emitted to **stdout** and `CliError` to **stderr** — one document per stream.
- The "post-wave preservation gap" (consensus) is **refuted**: merged chunks fold into
  the integration branch (loose branches deleted), blocked chunks are preserved before
  the code stage returns Ok, and the Err report is built from the already-accumulated
  `run.chunk_reports`, so every preserved sibling is carried on any post-spec error.
- **Applied:** `cmd_run` emits the report best-effort (never `?`) so an emit failure
  can't replace the pipeline error's exit code; removed the unused/foot-gun
  `From<PipelineFailure> for CliError`.
- **Deferred follow-up:** the Err report hardcodes `verify: None`. A post-verify hard
  failure therefore drops the verify evidence from the audit. Threading the latest
  verify state onto `Run` would restore it — a fidelity improvement orthogonal to the
  inv-5 audit that is this issue's goal.

## Invariants preserved

- Inv 5: preservation still holds — now it is also auditable. Teardown gating unchanged.
- Exit code: `From<PipelineFailure>` → the inner `PipelineError`'s system/user mapping;
  never downgraded to 0.
- Inv 1–4 untouched (this file is the additive pipeline driver; no event-append / lock
  / reducer paths).
