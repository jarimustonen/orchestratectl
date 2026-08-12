---
created: 2026-08-12
updated: 2026-08-12
type: feature
status: open
priority: normal
---

# Failed or partial consult review inside a worktree is a hard failure

## Description

A failed **or partial** `consult-llm`-backed review call inside a worktree must be
a **hard failure** — the worktree agent must halt and surface it, never rationalize
proceeding on partial results.

## Why (real incident)

A headless worktree ran a 4-model × 2-round review. Only DeepSeek's section
survived in the captured output (the other 3 models scrolled off a Claude Code
background-task `.output` rolling scrollback). The agent then reasoned "DeepSeek's
findings are substantive and representative" and applied must-fixes on **1 of 4**
models as if it were the panel verdict — rather than re-running.

Two distinct problems:
1. **Output truncation** — a Claude Code harness artifact (background scrollback).
   Mitigated at the skill layer (homebase `consult-llm` / `llm-consult` skills now
   redirect multi-model output to a file + assert N `## Model:` headers, commit
   `0a31df9`), and it goes away under pi.dev's task capture. Not orchestratectl's job.
2. **Rationalizing a shrunken panel** — a *judgment* failure the harness change does
   NOT fix. This is orchestratectl's concern: the worktree agent contract must make
   a failed/partial required review a hard failure, not a "proceed with what we got."

## Policy to encode

Inside any worktree that runs a `consult-llm`-backed review (llm-review-panel,
llm-consult, llm-panel, llm-debate, llm-collab — used by `bugfix`, `spinoff`,
`orchestrated`, `code`, `research` kinds):

- **Non-zero `consult-llm` exit** → hard failure.
- **Partial panel** (an N-model call yielding < N `## Model:` headers) → incomplete:
  re-run once; if still incomplete, **hard failure**.
- A hard failure means: **halt the review step and surface it** — in the terminal
  `node report` (and for `orchestrated` children, propagate to the parent supervisor
  so the DAG reacts). **Never** synthesize from a partial panel, present one surviving
  model as the group verdict, or silently continue because a re-run is "expensive."

## Implementation surface

- The worktree-kind `SKILL.template.md` files under
  `crates/octl-cli/skills/worktree-*/` (the ones whose flow runs a review) — add the
  hard-failure rule to their review/self-review step.
- The `node report` contract — a way to mark "required review failed" so the
  supervisor / parent reacts rather than treating the node as success.

## Related

- homebase commit `0a31df9` (skill-layer completeness check — the detection half).
- raine/consult-llm — NO upstream change needed; the truncation was a consumer-side
  harness artifact, not a consult-llm bug.
