---
created: 2026-08-12
updated: 2026-08-22
type: feature
status: done
priority: normal
lane: skills
lane_seq: 30
closed: 2026-08-22
commits:
- hash: 440116cf069a2e82faa6818a901c20daecef9cee
  summary: disclose incomplete worker subworkflows
---

# Surface actionable tool failures to the spawning agent

## Description

A worktree must not hide, rationalize, or silently downgrade a tool or required sub-workflow failure. The spawning agent needs an explicit, actionable account of what failed and enough bounded context to decide whether the work can continue, should be retried, or warrants a bug report.

This is guidance for the bundled worktree workflows, not a new supervisor failure state or node-report schema.

## Observed incident

A required four-model review produced only one surviving model section after consumer-side output truncation. The worker treated that partial result as representative and continued. The immediate capture problem was fixed elsewhere, but the general judgment failure remains: missing required output was not surfaced honestly to the spawning agent.

## Generic policy

When a tool, command, external service, or required review/sub-workflow fails or returns detectably incomplete output:

- never claim that the affected step completed successfully;
- retry only when the workflow already defines a bounded retry;
- distinguish a required-step failure from an optional/advisory failure;
- required-step failure stops that step, but does not automatically force the whole run into a generic failed state when a useful handoff is possible;
- optional failure may allow work to continue, but must still be disclosed in the final report;
- do not present partial output as the complete result or silently substitute one surviving source for a requested panel;
- propagate the failure to the spawning agent in the terminal report or blocked handoff.

## Existing communication channel

Reuse the existing terminal `node.report`; do not invent a second worker-to-spawner message path:

- completed work includes the failure disclosure in the report file passed to `taskfleet run merge --report-file`;
- work that cannot safely complete submits a direct blocked `taskfleet node report` with `success: false`, the failure disclosure, and any `recoverable_work` / `discussion_items` context;
- the spawning agent reads the same durable report through `run show` / `run wait` and decides whether to retry, recover, continue, or file a bug.

The report is communication, not an automatic severity verdict. A tool error alone must not create a new supervisor terminal state or bypass the existing typed outcome and work-preservation rules.

## Required failure context

The report should contain a concise `Tool/sub-workflow failure` section with enough information to assess and file a bug without rediscovery:

- tool or sub-workflow name and the purpose for which it was invoked;
- expected result or completeness condition;
- observed exit/error/incompleteness signal;
- bounded retry attempts and their outcomes;
- which task step is blocked or potentially unreliable;
- whether any work continued, and why that was safe;
- relevant bounded stderr/output excerpt and a stable artifact/log path when available;
- a suggested owner/surface for a possible bug report, without filing one automatically.

Secrets, credentials, personal data, and unbounded logs must not be copied into the report.

## Acceptance Criteria

- [x] Applicable bundled worktree workflow templates carry the generic disclosure rule.
- [x] A required failed or incomplete tool result cannot be described as completed.
- [x] The spawning agent receives the disclosure through the existing terminal report and enough structured prose to decide retry, recovery, or bug filing.
- [x] The rule permits a blocked/recoverable handoff instead of forcing every tool error to terminal failure.
- [x] Tests or snapshots cover required failure, optional failure with continuation, partial panel output, bounded retry exhaustion, and secret-safe context.
- [x] No new CLI event or node-report schema is introduced unless implementation proves prose cannot carry the requirement.
