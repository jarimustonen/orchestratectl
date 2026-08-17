---
name: worktree-bug-analysis
description: Spawn an autonomous READ-ONLY worktree via `orchestratectl run create --kind spinoff` that analyses ONE already-filed bug and writes its findings back into the issue — reproduce or explain the symptom, locate the responsible code (Read/Grep only), classify it (real bug / expected behaviour / cannot tell), estimate severity, and sketch what a fix would touch. Never changes application code; the only write is the issue update, which it self-merges. Use when an existing bug issue needs understanding before a fix/defer/not-a-bug decision. For fixing a bug use `/worktree-bugfix`; for open-ended multi-source research use `/worktree-research`.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-bug-analysis — read-only, issue-updating

Analyse ONE already-filed bug in an isolated worktree and record what you found
**in the issue itself** — no code changes, no fix, no new issue. This exists
because the two obvious tools don't fit: `worktree-research` refuses
debugging/bug-investigation topics, and `worktree-bugfix` *fixes* code and
creates a new issue. This skill sits between: it takes an **existing bug slug**,
investigates read-only, and updates that issue.

It is a `spinoff`-shaped run — one autonomous agent that self-merges and does not
pause for the user — so it spawns via `orchestratectl run create --kind spinoff`
with a read-only brief. It runs **headless** (its only output is an issue update;
nobody watches it), so a triage batch does not clutter the window list. Read
`orchestratectl-overview` first; read `worktree-spinoff` for the shared
autonomous-merge contract that this skill reuses verbatim.

## When to use

- ✅ You have an **existing** bug issue slug that needs understanding before a
  fix/defer/not-a-bug decision.
- ❌ Fixing the bug → `/worktree-bugfix`.
- ❌ Filing a new bug / any issue that does not yet exist — this skill never
  creates issues.
- ❌ Open-ended multi-source research → `/worktree-research`.

## Hard constraints

1. **Never change application code.** The only files this worker writes are the
   bug's own `issues/<slug>/item.md` (and, if useful, `issues/<slug>/analysis.md`).
   Editing anything under the product source tree is a *fix* — out of scope.
2. **Do not decide the disposition.** Classify and recommend; fix-now / defer /
   not-a-bug stays with the user. Do not close the issue or change its status or
   disposition labels.
3. **Autonomous, no user pause.** Like `worktree-spinoff`: investigate, update
   the issue, self-merge. Do NOT run `/wrap-up`.

## Workflow

### 0. Validate context

1. Working directory must be a git repo. Per repo CLAUDE.md, the current branch
   must be clean.
2. `orchestratectl version --output json` to confirm `{{CLI_VERSION}}` matches
   the running binary.
3. Capture the current branch as the source/merge target.

### 1. Resolve the bug slug

The remaining argument (after any agent/layout flags, parsed as `worktree-spinoff`
does) is the **bug issue slug**. It must already exist — if `issues/<slug>/item.md`
is missing, abort with a clear error. This skill does not create issues.

### 2. Build the read-only brief

The brief must be self-contained (a spinoff cannot ask follow-ups). It MUST carry
the read-only hard constraints above **and** end with the mandatory closing
`orchestratectl run merge` step (see "Terminal report" below). Include:

1. **Objective** — understand and scope the bug in `issues/<slug>/item.md`; do
   NOT fix it, do NOT change application code; write findings back into the issue.
2. **Steps**:
   - Read `issues/<slug>/item.md` **and every attachment** under
     `issues/<slug>/attachments/` (screenshots are often the whole report).
   - Reproduce or explain the behaviour; locate the responsible code path with
     **Read/Grep only**. If you cannot reproduce, say why.
   - Classify: **real bug** / **expected behaviour** / **cannot tell**. Estimate
     rough severity and who it hits. Sketch what a fix would touch (files/areas)
     — a sketch, not an implementation.
   - Write findings into `issues/<slug>/item.md` under `## Triage analysis` (or
     `## Suspected Root Cause`): verdict, severity, affected area, repro status,
     fix sketch. Keep it tight; for a long trace add `issues/<slug>/analysis.md`
     and link it.
   - Commit with plain `git` and a `Refs-Issue: <slug>` trailer.
3. **Done criteria** — the issue carries the analysis; the branch is committed
   and merged back; no application code changed.

Long brief → temp file + `--prompt-file <path>` (`mktemp -t bug-analysis-XXXXXX.md`).

### 3. Create the run

```
orchestratectl run create \
  --kind spinoff \
  --headless \
  --title "bug-analysis-<slug>" \
  --prompt-file <brief-file> \
  [--source-branch <branch>]
```

- `--kind spinoff` + `--title` + `--prompt-file` (or `--task`) required — same
  flag rules as `worktree-spinoff`.
- `--headless` is the default for this skill: the run's only deliverable is an
  issue update, so its window belongs in the detached `headless` session, not the
  user's window list. Auto-cleanup still closes it on terminal. Drop `--headless`
  only if the user explicitly wants to watch the analysis live.
- `--source-branch` defaults to the current branch from step 0.

### 4. Success envelope

Same shape as `worktree-spinoff` (`run_id`, `supervisor`, `kind: spinoff`,
`node_id`, `tmux_window`, `worktree_path`, `branch`). Read `data.run_id`; if
`data.supervisor` is `null` or `{"note": "..."}`, surface it and stop.

### 5. Report to the caller

- Run id, branch, and the issue path `issues/<slug>/item.md`.
- That the worker self-merges the issue update via `orchestratectl run merge` — no
  `/worktree-merge` handoff.
- Follow progress with `orchestratectl run show <run-id>` or
  `orchestratectl run wait <run-id>`; the headless window is reachable with
  `tmux attach -t headless` if the user wants to watch.

When spawned by another skill rather than invoked directly by the user, return the
structured payload (run id, node id, branch) to the caller instead of a human summary.

## Terminal report (mandatory)

Closing is **one call** — identical to `worktree-spinoff`/`worktree-research`.
`orchestratectl run merge` rebases + merges the branch into its source and submits
the terminal `node report` in the same call; until it runs the run stays
`pending` and the window never closes. The brief MUST instruct the worker to run
this once the issue update is committed:

1. **Discover the run id** from inside the worktree:

   ```bash
   short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
   run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
   ```

2. **Write the §7.3 payload** to a temp file — `success: true`, `summary` = the
   one-line verdict (e.g. "real bug, medium sev, in <area>"), the three arrays
   usually empty (a fix worth doing goes in `spinoff_proposals[]` with
   `proposed_kind: "bugfix"`):

   ```bash
   cat > /tmp/node-report-${run_id}.json <<'JSON'
   { "success": true, "summary": "<verdict>", "discussion_items": [], "spinoff_proposals": [], "wrap_up_recommendations": [] }
   JSON
   ```

3. **Merge and report in one call:**

   ```bash
   orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json
   ```

   On a clean merge the supervisor consumes the report and tears down the
   worktree/window/branch — do not run any manual tmux/git cleanup. On a merge
   conflict it exits non-zero with `error.code: "merge_failed"` and submits no
   report; resolve (or `/complex-rebase`) and re-run the same call. **Do not**
   close the issue or change its status or disposition labels — that decision is the
   user's.

## Non-goals

- Does NOT fix bugs or touch application code — that's `/worktree-bugfix`.
- Does NOT create issues — it only updates an existing one.
- Does NOT decide fix/defer/not-a-bug, close the issue, or change its status or
  disposition labels.
- Does NOT run open-ended multi-source research — that's `/worktree-research`.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. On the first
invocation in a session, run `orchestratectl version --output json`, compare
`.data.version` to `{{CLI_VERSION}}`: **Missing** → install; **Older** → tell the
user to upgrade and stop; **Newer** → `orchestratectl skill install --force`;
**Equal** → proceed.

## Example

```
/worktree-bug-analysis checkout-total-off-by-one
```
