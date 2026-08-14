---
created: 2026-07-25
updated: 2026-08-14
type: task
status: obsolete
priority: normal
related: ['@pipeline-walking-skeleton']
closed: 2026-08-14
closed_by: agent-cut-pipeline-floor-harness-heavy
---

# Pipeline hardening: sandbox spec/verify, atomic source-ref CAS, typed statuses

## Description

Follow-ups from the T5 /llm-review that are beyond the walking-skeleton scope (the skeleton already validates the harness commit, restores the worktree after spec/verify to discard their side effects, merges the exact gated OID, splits the converge/merge tiers, and preserves unmerged work). Remaining hardening: (1) Sandbox spec/verify instead of relying on post-hoc restore_to — run them read-only / network-restricted so a prompt-injected intent or repo file cannot exfiltrate ambient/SOPS credentials; the code stage keeps write access. (2) Atomic source merge: build the final merge candidate in a pipeline-owned worktree from the pinned source OID and update the source ref via 'git update-ref <ref> <new> <expected-old>' (compare-and-swap), detecting a source-branch move between gate and merge (TOCTOU). (3) Replace stringly-typed PipelineReport.status / ChunkReport.outcome with enums for machine consumers. (4) Spec retry should feed the T2 validator error text back to the model, and treat malformed-JSON (not just invalid-plan) as a retryable candidate. (5) Pass the claude prompt via stdin instead of argv (ARG_MAX + process-listing exposure). (6) Strict typed verify-response parsing (serde deny_unknown_fields). (7) Share a CARGO_TARGET_DIR across chunk worktrees so the floor's cargo test/clippy captures don't rebuild from scratch per chunk. (8) Default workdir uniqueness (run id) to avoid same-slug collisions.

## Resolution

### 2026-08-14T04:42:34Z · @agent-cut-pipeline-floor-harness-heavy

Superseded by the 0.2 subtractive cut (cut-pipeline-floor-harness-heavy): the code-pipeline subsystem (pipeline/*, floor/*) and the harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek) it targeted were deleted. Nothing left to harden/wire/triage. See docs/decisions/0001-thin-supervisor-vs-harden.md D3.
