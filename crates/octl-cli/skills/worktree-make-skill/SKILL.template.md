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
  `orchestratectl event tail <run-id> --follow`.

## Terminal report (mandatory)

Closing the run is **one call**. `orchestratectl run merge` owns the
whole closing step: it rebases + merges the worktree branch into its
source branch, and submits the terminal `node report` itself (stamped
`via: "explicit-merge"`). There is no longer a separate
`/worktree-merge`-then-`node report` two-step. Until that report lands
the per-run supervisor keeps polling, `orchestratectl run show` reads
`status: pending`, and the tmux window never closes — so the merge
call IS what releases the run.

Because a make-skill run authors a substantial skill and runs a
multi-round review, its wrap-up and discussion items matter. The brief
MUST instruct the agent to build a §7.3 `--report-file` payload and
pass it to the merge, **as its final action before the session ends**:

1. **Discover the run id** from inside the worktree. The branch is
   `wt/<short>-<slug>`, where `<short>` is the first 10 alphanumerics
   of the run id:

   ```bash
   short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
   run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
   ```

   The terminal node defaults to `n-0001` (a single-worker kind always
   has exactly one node); override with `--node-id` only if needed.

2. **Write the §7.3 payload** to a temp file. These exact field names are
   what the supervisor consumes — do NOT use `discuss`,
   `spinoff_candidates`, or `wrap_up`: an unknown key still passes
   validation, but its contents are silently dropped.

   ```bash
   cat > /tmp/node-report-${run_id}.json <<'JSON'
   {
     "success": true,
     "summary": "<one-line outcome>",
     "discussion_items": [],
     "spinoff_proposals": [],
     "wrap_up_recommendations": []
   }
   JSON
   ```

   - `success` — **required** boolean. `true` when the work merged
     cleanly; `false` when reporting a blocked or failed outcome.
   - `summary` — optional one-line human-readable result.
   - `discussion_items[]` — decisions that genuinely needed a human
     call. Each: `{"topic": "<non-empty>", "severity":
     "discuss|critical|info", "options": ["…"]}`.
   - `spinoff_proposals[]` — follow-up work worth spawning. Each:
     `{"proposed_title": "<non-empty>", "proposed_kind":
     "spinoff|code|research|bugfix|technical-decision|make-skill|fan-out|orchestrated",
     "rationale": "<why>"}`.
   - `wrap_up_recommendations[]` — array of strings; advice for the
     caller (further reviews, doc updates, additional siblings).

   Even a clean, no-follow-up run submits `{"success": true}` with the
   arrays empty — the merge call itself is what releases the supervisor.

3. **Merge and report in one call:**

   ```bash
   orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json
   ```

   The `--report-file` payload is validated **before** the merge, then
   the branch is rebased + merged into its recorded `source_branch`
   (override with `--source <branch>`; falls back to main/master
   auto-detect), and the terminal report is submitted carrying the rich
   discussion / spinoff / wrap-up items. This is the agent's **final
   action**. On a clean merge the per-run supervisor winds the run down
   and tears down the worktree, tmux window, and branch automatically —
   do **not** manually run tmux/git cleanup.

   On a merge conflict or failure `run merge` exits non-zero with
   `error.code: "merge_failed"` and submits **no** report — the node
   stays live. Resolve the conflict (or run `/complex-rebase` for a
   non-trivial divergence) and re-run the same `run merge` call.

This step is **not optional**. A successful merge with no report leaves
the run dangling, with no structured outcome for the caller to read.

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
