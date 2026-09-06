---
created: 2026-07-26
updated: 2026-08-14
type: task
status: obsolete
priority: normal
related: ['@pipeline-circuit-breakers']
closed: 2026-08-14
closed_by: agent-cut-pipeline-floor-harness-heavy
---

# Pipeline breakers: in-flight interruption + spec/verify token metering (T6 follow-ups)

## Description

Deferred follow-ups from pipeline-circuit-breakers (T6, design §9), from the 4-model llm-review (see that issue's worktree history/review-pipeline-circuit-breakers.md + assessment-*.md).

The five §9 breakers landed as DETERMINISTIC, supervisor-owned, POST-ATTEMPT backstops (metered/checked between synchronous agent invocations). Two enhancements were deferred to keep the diff inside pipeline/harness:

1. True in-flight kill-switch. A single agent call can overshoot a cost/token/wall-time ceiling before it returns; the breaker aborts before the NEXT call. A hard in-flight stop needs a run-level deadline propagated into the harness CancelToken (+ per-subprocess process-group kill) and streaming-usage cancellation. A centralized invoke_agent wrapper (reserve process -> deadline -> call -> meter -> check) would also consolidate the scattered post-call checks.

2. Spec/verify token/cost metering. SpecProvider/VerifyProvider return Value/VerifyJudgment and do NOT surface Usage, so spec+verify (the Opus-dominant spend, design §11) count only toward process-count — the cost/token breakers see $0 for them. Extend the provider traits to return Usage and fold it into the tally.

Lower-priority: harness artifacts under $TMPDIR/taskfleet-harness are outside the storage-cap measurement; typed BreakerReason enum instead of Option<String>; per-run unique workdir (ULID); failure_counts reset/scope on re-spec.

## Resolution

### 2026-08-14T04:42:34Z · @agent-cut-pipeline-floor-harness-heavy

Superseded by the 0.2 subtractive cut (cut-pipeline-floor-harness-heavy): the code-pipeline subsystem (pipeline/*, floor/*) and the harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek) it targeted were deleted. Nothing left to harden/wire/triage. See docs/decisions/0001-thin-supervisor-vs-harden.md D3.
