---
name: octl-spawn-spinoff
description: Spawn an autonomous spinoff worktree via orchestratectl — a single fire-and-forget agent that takes a focused task, executes it in its own git worktree, and merges itself back. Use when the user wants a parallel sub-task handled without interactive review.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# octl-spawn-spinoff

A **spinoff** is one autonomous agent run in its own git worktree, doing
one well-scoped task, and merging itself back to the source branch when
done. No interactive review. The canonical way to launch one is via
`orchestratectl`, not by hand-crafting branches.

## Invocation

```
orchestratectl run create \
  --kind spinoff \
  --title "<2–4 word slug>" \
  --task "<the task in one paragraph>" \
  --source-branch <branch>
```

(The default is `--output jsonl` — one compact envelope per line on
stdout. Add `--output text` for a human-readable summary or `--output
json` for pretty-printed JSON.)

- `--kind spinoff` is required; it picks the autonomous worker recipe.
- `--title` is required; a short slug that names the run, the branch
  (`wt/<short>-<title>`), and the tmux window.
- `--task` is the *entire* brief the spinoff agent will see. It must
  be self-contained — the spinoff does not share your conversation
  history. State the goal, the constraints, and what "done" looks like.
  For a long brief, write it to a file and pass `--prompt-file <path>`
  instead of inlining via `--task`.
- `--source-branch` is the branch the worktree forks from, and the
  branch the spinoff merges itself back into when done. Default is the
  current branch when omitted.

## Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run": {
      "id": "01HZ...",
      "kind": "spinoff",
      "lifecycle": "autonomous",
      "status": "pending",
      "source_branch": "main",
      "target_branch": "main"
    }
  }
}
```

`lifecycle` is the run's category (`autonomous` for spinoffs) and never
changes. `status` is the progress field: starts at `pending`, transitions
to `running` once the worker picks it up, and reaches a terminal value
(`done | failed | cancelled`) when the run settles. Branch on `status`
to detect completion. Use `orchestratectl run show <id>` to follow
progress (see the `octl-run-overview` skill for the response shape).

## When to use a spinoff vs. other variants

- **Spinoff** — one focused autonomous task, no review. "Update all
  docstrings in module X." "Refactor helper Y into its own crate."
- **Interactive worktree** — user wants a hands-on, human-driven session:
  add `--interactive` to `run create` so the supervisor waits for an
  explicit `run merge`/`run cancel` (a default spinoff is always headless +
  autonomous). Not this skill's default path.
- **Fan-out** — N similar independent units (≥5). Not this skill — use
  `run create --kind fan-out`.

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

## Decision forks

An autonomous worker never blocks on an interactive prompt. At a genuine fork,
it records `node.awaiting_input` with a non-empty `discussion_items` array whose
items carry `topic`, string `options`, and `recommended_default`. It then either
resolves the marker with `node.input_resolved` and follows that default after a
bounded five-minute wait, or submits a terminal `success:false` blocked report
with the same discussion items. The signal is visible immediately and propagates
to `run wait` / the registered notify hook after the grace window.

## Errors

Standard error envelope on stderr, non-zero exit. Likely codes:

- `invalid_argument` — bad branch name, missing `--task`/`--prompt-file`
- `branch_not_found` — `--source-branch` does not exist locally
- `worktree_create_failed` — git refused (uncommitted changes,
  conflicting worktree)

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. On the
first invocation in a session, run
`orchestratectl version --output json`, parse the JSON, and read
`.data.version`. Compare it to `{{CLI_VERSION}}`:

- **Missing**: install one of:
  - **Homebrew** (macOS/Linux): `brew install jarimustonen/orchestratectl/orchestratectl`
  - **Cargo** (any platform with a Rust toolchain): `cargo install orchestratectl`
  - **Shell installer** (no toolchain):
    `curl -LsSf https://github.com/jarimustonen/orchestratectl/releases/latest/download/orchestratectl-installer.sh | sh`

  (Publishing channels are TBD; the placeholders above mirror
  `issuectl` conventions and will be replaced once the release pipeline
  ships.)
- **Older than `{{CLI_VERSION}}`**: tell the user the skill expects
  `{{CLI_VERSION}}` and suggest upgrading via the same channel they
  originally used (`brew upgrade jarimustonen/orchestratectl/orchestratectl`,
  `cargo install orchestratectl --force`, or re-run the shell
  installer). Stop and wait — the `run create --kind spinoff` flag
  surface may have changed.
- **Newer than `{{CLI_VERSION}}`**: the installed binary is ahead of
  what this skill was written for. The whole bundled skill catalog has
  moved with the binary, so refresh all of them:
  `orchestratectl skill install --force` (add `--agent codex` for Codex
  or `--agent all` for both). To refresh only this skill, run
  `orchestratectl skill install octl-spawn-spinoff --force`. Continue
  once the skills match.
- **Equal**: proceed normally.
