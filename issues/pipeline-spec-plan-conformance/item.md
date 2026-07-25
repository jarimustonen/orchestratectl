---
created: 2026-07-25
updated: 2026-07-25
type: bug
status: fixed
priority: high
closed: 2026-07-25
---

# pipeline: spec stage produces schema-invalid plan.json (missing acceptance); blind retry doesn't repair — add schema-complete prompt + validation-error repair loop

## Description

## Agent Runs

### 2026-07-25T06:02:35Z · @claude

Fixed: schema-complete spec prompt (build_spec_prompt + plan_schema_requirements, enumerating every REQUIRED plan.json field + acceptance >=1 executable check, still embedding the drift-guarded filled example) and a validation-error repair loop (produce_and_validate_plan + SpecProvider::repair_plan + build_repair_prompt) that feeds the exact validator error + the invalid JSON back to the model. Strict parse preserved (normalize_plan only overwrites supervisor-owned fields; no server-side patch of chunks/acceptance). On exhaustion the raw invalid plan is persisted to <workdir>/plan.invalid.json and the last validator message surfaced. Tests: repair_loop_feeds_validator_error_back_and_succeeds, persistently_invalid_plan_fails_with_raw_persisted_and_error_surfaced, repair_call_failure_persists_the_prior_invalid_plan.

/llm-review (gemini,openai,anthropic,deepseek) run; report at history/review-pipeline-spec-plan-conformance.md. Actioned 4 consensus findings: fenced rejected-JSON + validator-error as DATA in the repair prompt (prompt-injection), persist prior invalid plan on repair-call transport failure, made repair_plan a required trait method (blind-retry default would re-introduce the bug), corrected the 'cannot drift' doc overclaim.

Deferred follow-ups (out of tight scope / pre-existing): syntax-error repair via a text-returning trait; restore_to before each spec attempt; stderr in claude-exit errors; --model pinning/provenance; sandboxing --dangerously-skip-permissions; ARG_MAX->stdin prompt transport; PlanInvalid classified as user error. MAX_PLAN_ATTEMPTS kept at 2 per 'keep the existing count'.
