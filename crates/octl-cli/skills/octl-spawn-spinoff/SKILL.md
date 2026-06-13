---
name: octl-spawn-spinoff
description: Spawn an autonomous spinoff worktree via orchestratectl — a single fire-and-forget agent that takes a focused task, executes it in its own git worktree, and merges itself back. Use when the user wants a parallel sub-task handled without interactive review.
version: 1
---

# octl-spawn-spinoff

> ## ⚠️ PREVIEW — DO NOT INVOKE BLINDLY
>
> The `orchestratectl run create --kind spinoff` surface documented here
> is **not yet implemented**. It lands in the `all-kinds-spawn` issue.
> Until then:
>
> 1. Call `orchestratectl --help` and confirm the `run` subcommand is
>    listed. If it is not, **do not attempt the invocation below.**
> 2. If the user is in a Claude Code environment, invoke the
>    `/worktree-spinoff` slash-command skill instead (it predates this
>    CLI and is the working path today).
> 3. Otherwise, tell the user the spinoff surface is not yet shipped and
>    stop.
>
> The rest of this file documents the forward contract so that, on the
> day the CLI lands, agents can target the right invocation without a
> skill update lagging behind.

A **spinoff** is one autonomous agent run in its own git worktree, doing
one well-scoped task, and merging itself back to the source branch when
done. No interactive review. The canonical way to launch one is via
`orchestratectl`, not by hand-crafting branches.

## Invocation

```
orchestratectl run create \
  --kind spinoff \
  --prompt "<the task in one paragraph>" \
  --source-branch <branch> \
  [--target-branch <branch>]
```

(The default is `--output jsonl` — one compact envelope per line on
stdout. Add `--output text` for a human-readable summary or `--output
json` for pretty-printed JSON.)

- `--kind spinoff` is required; it picks the autonomous worker recipe.
- `--prompt` is the *entire* brief the spinoff agent will see. It must
  be self-contained — the spinoff does not share your conversation
  history. State the goal, the constraints, and what "done" looks like.
- `--source-branch` is the branch the worktree forks from. Default is
  the current branch when omitted.
- `--target-branch` is where the spinoff merges back. Defaults to
  `--source-branch`.

## Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run": {
      "id": "01HZ...",
      "kind": "spinoff",
      "lifecycle": "pending",
      "status": "queued",
      "source_branch": "main",
      "target_branch": "main"
    }
  }
}
```

The run starts in `lifecycle: pending` and transitions to `running`
once the worker picks it up. Use `orchestratectl run show <id>` to
follow progress (see the `octl-run-overview` skill for the response
shape).

## When to use a spinoff vs. other variants

- **Spinoff** — one focused autonomous task, no review. "Update all
  docstrings in module X." "Refactor helper Y into its own crate."
- **Worktree-code** — user wants to review the diff interactively. Not
  this skill.
- **Fan-out** — N similar independent units (≥5). Not this skill — use
  `run create --kind fan-out`.
- **Orchestrate** — heterogeneous, dependency-ordered features. Not
  this skill.

## Writing the prompt

The prompt is the most failure-prone input. The spinoff cannot ask you
follow-up questions. Include:

1. **Goal** — one sentence on what to deliver.
2. **Context** — files, modules, or constraints that matter. Quote
   paths.
3. **Done criteria** — concrete, verifiable. "All tests pass" or "no
   `clippy::pedantic` warnings introduced."
4. **Quality bar** — does the spinoff need to run `/llm-review`? Should
   it merge itself or hand off?

If any of those are missing, ask the user before spawning. A spinoff
that misinterprets the task wastes a worktree and a merge cycle.

## Errors

Standard error envelope on stderr, non-zero exit. Likely codes:

- `invalid_argument` — bad branch name, missing `--prompt`
- `branch_not_found` — `--source-branch` does not exist locally
- `worktree_create_failed` — git refused (uncommitted changes,
  conflicting worktree)
- `not_implemented` — the spinoff kind is not yet wired up in this
  build; fall back to `/worktree-spinoff`
