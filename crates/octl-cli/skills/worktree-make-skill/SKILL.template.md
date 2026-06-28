---
name: worktree-make-skill
description: Spawn an autonomous worktree via `orchestratectl run create --kind make-skill` to author a SUBSTANTIAL new Claude Code skill with multi-round LLM review, then self-merge. Use when the skill is substantial — composes other skills via `/<name>` calls, will be used in high-fan-out workflows, or needs a multi-round review pass before landing. For one-shot small skills use `/skill-creator` inline; for reviewing an existing skill use `/llm-skill-review`.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-make-skill

A **make-skill worktree** is one autonomous agent whose deliverable is
a new `SKILL.md` (and any companion scripts) committed into the
target skill catalog, with at least one round of `/llm-skill-review`
applied before merge. Same autonomous self-merge contract as
`worktree-spinoff`, plus a built-in review loop.

Read `orchestratectl-overview` first; read `worktree-spinoff` for the
shared autonomous-merge contract.

## When to use

- ✅ The new skill is substantial — composes other skills, will be
  used in high-fan-out workflows, or needs review before landing.
- ✅ The user explicitly asked to "author a new skill in a worktree"
  or "make a skill for X" where X is non-trivial.
- ❌ One-shot small skill that does not warrant multi-round review →
  invoke `/skill-creator` inline; do not spawn a worktree.
- ❌ Reviewing or refactoring an EXISTING skill → `/llm-skill-review`.
- ❌ Authoring a binary-bundled `orchestratectl` skill — those land in
  `crates/octl-cli/skills/<name>/SKILL.template.md` via a normal
  `/worktree-code` cycle so the catalog test and snapshots can update
  in the same commit.

## Workflow

### 0. Validate context

1. Working directory must be a git repo with a clean current branch.
2. Skill target directory must exist (typically
   `~/.claude/skills/` for global, or `<repo>/.claude/skills/` for
   repo-local). The agent needs to know where the SKILL.md will
   land.
3. `orchestratectl version --output json` to confirm
   `{{CLI_VERSION}}`.

### 1. Sharpen the skill brief

Substantial skills fail when their *trigger conditions* are vague. Pin
down:

- **Skill name** — kebab-case slug; this becomes the directory and the
  frontmatter `name:`.
- **One-line `description:`** — the agent's first read; must encode
  trigger conditions, scope boundaries, and explicit "do NOT use
  for X / use Y instead" anti-patterns.
- **Compositions** — which existing skills this new one calls (with
  `/<name>` references the new skill must include "READ THEM" hints
  for).
- **Output / side effects** — what the skill writes to disk, what it
  pushes to remote services, what it never touches.
- **Review bar** — minimum one `/llm-skill-review` round; the brief
  should require revising-until-no-finding-survives or one bounded
  round depending on scope.

If any of the above is missing, ask **once** before spawning.

### 2. Build the prompt

The agent runs autonomously. Brief includes:

1. Sharpened skill brief from step 1.
2. Target directory and file path
   (`<target-dir>/<skill-name>/SKILL.md`).
3. Companion files allowed (scripts, fixtures); structure
   constraints.
4. **Review loop** — after first draft, run `/llm-skill-review` on the
   SKILL.md, apply findings, decide autonomously whether a second
   round adds signal, loop until stable.
5. **Done criteria** — file exists, committed, merged. The skill
   passes its own trigger-fit lens (an agent reading just the
   description would correctly invoke or not invoke it for the
   intended cases).

### 3. Create the run

```
orchestratectl run create \
  --kind make-skill \
  --title "<skill-name>" \
  --task "<self-contained brief>" \
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
    "kind": "make-skill",
    "lifecycle": "autonomous",
    "tmux_window": "🛠 wt/<skill-name>",
    "worktree_path": "/Users/<you>/.../worktrees/<skill-name>",
    "branch": "wt/<skill-name>"
  }
}
```

### 5. Report to the caller

Tell the user:

- Run id, branch, tmux window, target file path.
- That the worktree self-merges after the review loop converges.
- How to follow: `orchestratectl run show <run-id>`,
  `orchestratectl event tail --run <run-id> --follow`.

## Issue Management

Same as `worktree-spinoff`: skip in driver mode; when issue-driven,
the agent records commits / closes the issue itself.

## Errors

Same envelope and codes as `worktree-spinoff`. One make-skill specific
note: if `/llm-skill-review` cannot reach a model (network, auth), the
run will stall on the review loop. Surface the symptom from
`event tail` rather than guessing.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. On the
first invocation in a session, compare `.data.version` from
`orchestratectl version --output json` to `{{CLI_VERSION}}`:

- **Missing**: install via Homebrew / Cargo / shell installer.
- **Older**: ask the user to upgrade; stop.
- **Newer**: `orchestratectl skill install --force` (or just
  `worktree-make-skill --force`).
- **Equal**: proceed.

## Example

```
/worktree-make-skill Author /deploy-canary skill — composes /verify and /code-review, blocks on dashboard checks, supports rollback
```
