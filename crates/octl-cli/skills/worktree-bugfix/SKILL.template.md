---
name: worktree-bugfix
description: End-to-end bug investigation worktree. First creates an `issuectl` issue from the bug report on the current branch (committed before the worktree spawns), then spawns an autonomous worktree via `orchestratectl run create --kind bugfix` that investigates, classifies the fix as light vs. complex, runs the appropriate review skill, and merges itself back if no user input is required. Use when a teammate reports a bug or unexpected behavior and you want the whole investigate→fix→review→merge cycle handled in one go.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-bugfix

A **bugfix worktree** is one autonomous agent whose full job is:
investigate the reported bug, classify the fix as light (mechanical) or
complex (logic / cross-cutting), run the right review skill
(`/code-review` for light, `/llm-review` or `/llm-panel` for complex),
apply fixes, and merge itself back — same self-merge contract as
`worktree-spinoff`. The wrapper around it also opens an `issuectl`
issue on the current branch before spawning, so the bug is tracked
regardless of how the run terminates.

Read `orchestratectl-overview` first; read `worktree-spinoff` for the
shared autonomous-merge contract.

## When to use

- ✅ A teammate (or the user) reported a bug or unexpected behavior
  and you want the whole investigate→fix→review→merge cycle handled
  in one go.
- ✅ The user said "fix this bug for me", "investigate why X is
  happening and patch it", or pasted a reproduction.
- ❌ Vague "the app feels slow" without a reproducer → ask first;
  consider `/worktree-research` if it needs investigation but no fix
  yet.
- ❌ Known-cause one-line fix the user already has in mind → just edit
  the code; this skill is overkill.

## Workflow

### 0. Validate context

1. Working directory must be a git repo with a clean current branch.
2. `issuectl` must be on `PATH`; this skill creates the tracking
   issue before spawning the worktree. If `issuectl` is missing,
   refuse — bugfix runs without an issue lose context too easily.
3. `orchestratectl version --output json` to confirm
   `{{CLI_VERSION}}`.

### 1. Open the tracking issue first

On the current branch, before spawning anything:

```
issuectl --json new \
  --type bug \
  --title "<one-line bug summary>" \
  --body @<temp-file-with-full-report>
```

Capture the returned slug. The issue commit lands on the current
branch (per repo CLAUDE.md: commit immediately, do not leave
uncommitted state across worktree spawns). If `git status --porcelain`
is non-empty after `issuectl new`, commit it before the next step.

The bug report body should include:

- Symptom / reproduction.
- Expected vs actual.
- Environment (OS, version, branch, commit).
- Any logs / stack traces.
- Files / modules suspected (if obvious).

### 2. Build the prompt

The agent investigates first, so the brief is lighter on "do X" and
heavier on "find the cause, then decide":

1. Bug report — reproduce verbatim from the issue body.
2. **Investigation expectation** — read suspected files, run a
   reproducer if one is available, narrow to a root cause before
   editing.
3. **Classification** — once the cause is known, classify the fix:
   - **Light** — mechanical / single-file / no semantic change to
     other call sites. Review with `/code-review` (fast, focused on
     correctness + cleanup of the diff).
   - **Complex** — logic, cross-cutting, security/perf-relevant, or
     touches contracts. Review with `/llm-review` (full multi-model
     pass); switch to `/llm-panel` for design-shaped fixes.
4. **Apply + commit** — `/git-commit` per logical change.
5. **Issue management** — record commits and close the issue on
   completion.
6. **Self-merge** — merge to source branch on success; stop and
   surface to the user on conflicts (do not force).

### 3. Create the run

```
orchestratectl run create \
  --kind bugfix \
  --title "<bug slug>" \
  --task "<self-contained brief referencing issue <slug>>" \
  [--source-branch <branch>] \
  [--idempotency-key <key>]
```

Same flag rules as `worktree-spinoff`. Output defaults to
`--output jsonl`.

### 4. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ...",
    "supervisor": 12345,
    "kind": "bugfix",
    "lifecycle": "autonomous",
    "tmux_window": "🐛 wt/<bug-slug>",
    "branch": "wt/<bug-slug>"
  }
}
```

### 5. Report to the caller

Tell the user:

- Issue slug (already committed on the source branch).
- Run id, branch, tmux window.
- That the bugfix worktree self-merges; if it cannot reproduce or
  classify, it will stop and surface the reason via the run's event
  log.
- How to follow: `orchestratectl run show <run-id>` and
  `event tail --run <run-id> --follow`.

## Issue Management

The wrapper creates the issue **before** spawning; the agent itself
records commits and closes the issue on completion:

- `issuectl --json update <slug> --add-commit "<sha>:<summary>"`
- `issuectl --json update <slug> --status in-progress`
- `issuectl --json close <slug> --status fixed`

## Errors

Same envelope and codes as `worktree-spinoff`. Bugfix-specific
behavior:

- If the agent cannot reproduce, it should NOT close the issue and
  should not merge a speculative fix; it stops with the run in
  `lifecycle: completed` but `node report.success: false` plus a
  `discuss[]` item describing the gap.
- If classification slips (light → complex mid-stream), the agent
  re-routes to the heavier review automatically; no manual
  intervention needed.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. Compare
`.data.version` from `orchestratectl version --output json` to
`{{CLI_VERSION}}`:

- **Missing**: install via Homebrew / Cargo / shell installer.
- **Older**: ask the user to upgrade; stop.
- **Newer**: `orchestratectl skill install --force` (or just
  `worktree-bugfix --force`).
- **Equal**: proceed.

## Example

```
/worktree-bugfix Cancel sometimes leaves child node in 'running' when ledger probe races
```
