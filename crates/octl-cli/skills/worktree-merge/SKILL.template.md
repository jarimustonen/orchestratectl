---
name: worktree-merge
description: Merge a completed orchestratectl worktree branch back to its source/parent branch and tear the worktree down — branch rebased + merged, terminal `node report` submitted, and the tmux window + worktree + branch removed by the supervisor, all in ONE `orchestratectl run merge` call. Use when an autonomous worktree (spinoff, research, technical-decision, or a fan-out unit) reaches its merge-and-report step. Replaces the old two-step `/worktree-merge` + `orchestratectl node report` sequence. For a feature branch and main that have both diverged so far an ordinary rebase fails, recover with `/complex-rebase` then re-run.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-merge

Merge a worktree run's branch back to its source branch and tear the
worktree down — in one step. `orchestratectl run merge` owns the whole
lifecycle:

1. **Rebase + merge** the worktree branch onto its source (via the bundled
   merge backend — the same rebase/`flock`/`workmux merge` mechanics the
   old homebase `merge.sh` used; concurrent merges from `/fan-out` units
   are still serialized by the cross-worktree lock).
2. **Submit the terminal `node report`** stamped `via: "explicit-merge"`,
   so the per-run supervisor winds the run down.
3. **Tear down** — the supervisor closes the tmux window, removes the
   worktree, and deletes the branch on the terminal transition. Nothing
   for you to clean up by hand.

This replaces the old two-step dance (`/worktree-merge` to merge, then a
separate `orchestratectl node report` to release the supervisor). One call
now does both.

If you have not read it, read the `orchestratectl-overview` skill first —
it defines the run / supervisor / node vocabulary this skill assumes.

## When to use

- ✅ An autonomous worktree (spinoff, research, technical-decision, or a
  fan-out unit) has finished its work and committed, and now needs to
  merge-and-report. The driver/worker skills point here for their closing
  step.
- ❌ The branch and its source have diverged so far an ordinary rebase
  cannot reconcile them → run `/complex-rebase` first, then come back.
- ❌ You are NOT inside a worktree run managed by orchestratectl (no
  `~/.orchestratectl/runs/<id>/` for this branch) → this is a plain git
  branch; use `/git-rebase` + a normal merge instead.

## Workflow

### 0. Validate context

1. Confirm you are inside a git worktree on a non-`main`/`master` branch
   (`git rev-parse --abbrev-ref HEAD`). `run merge` refuses to merge
   main into itself.
2. Commit first. The merge refuses on an uncommitted working tree — run
   `/git-commit` if `git status --porcelain` is non-empty.
3. `orchestratectl version --output json` once per session; compare
   `.data.version` to `{{CLI_VERSION}}` (see "Install or upgrade").

### 1. Discover the run id

`run merge` takes the run id. Derive it from the branch — the branch is
`wt/<short>-<slug>`, where `<short>` is the first 10 alphanumerics of the
run id:

```bash
short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
```

If that yields nothing, this branch is not an orchestratectl-managed run —
stop and tell the user; do not improvise a merge.

### 2. (Optional) Write a structured report

A simple unit can skip this — `run merge` submits a minimal
`{"success": true, "summary": "..."}` report on its own.

But if you have decisions a human should see, follow-up work worth
spawning, or wrap-up advice for the caller (orchestrated children and
research/bugfix worktrees usually do), write the §7.3 payload to a temp
file and pass it with `--report-file`. `run merge` stamps it
`via: "explicit-merge"` and submits it in the same call. These exact field
names are what the supervisor consumes — an unknown key passes validation
but its contents are silently dropped:

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

- `success` — **required** boolean. `true` for a clean merge.
- `summary` — optional one-line human-readable result.
- `discussion_items[]` — decisions that needed a human call. Each:
  `{"topic": "<non-empty>", "severity": "discuss|critical|info",
  "options": ["…"]}`.
- `spinoff_proposals[]` — follow-up work worth spawning. Each:
  `{"proposed_title": "<non-empty>", "proposed_kind":
  "spinoff|research|technical-decision|fan-out",
  "rationale": "<why>"}`.
- `wrap_up_recommendations[]` — array of strings; advice for the caller.

### 3. Merge

```bash
orchestratectl run merge "$run_id" \
  [--source <branch>] \
  [--report-file /tmp/node-report-${run_id}.json]
```

Flag rules:

- `--source <branch>` — the merge target. Omit it and `run merge` uses the
  run's recorded `source_branch` (the branch the worktree was spawned
  from — usually `main`), falling back to main/master auto-detection.
  Pass `--source` only to override.
- `--report-file <path>` — the §7.3 payload from step 2. Omit for a
  minimal auto-report.
- `--node-id <id>` — defaults to `n-0001`; a single-worker run never needs
  this.
- `--dry-run` — resolve and validate inputs (branch, source, report file)
  and print the plan without merging or appending anything. Use it to
  sanity-check a tricky `--source`/`--report-file` before committing.
- Output defaults to `--output jsonl` — one compact envelope per line.

### 4. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ...",
    "node_id": "n-0001",
    "branch": "wt/<short>-<slug>",
    "source": "main",
    "merged": true,
    "report_seq": 7
  }
}
```

`merged: true` plus a `report_seq` means the branch landed and the terminal
report is recorded. The supervisor closes the tmux window, removes the
worktree, and deletes the branch within a second or two — your session
ends naturally as the window closes. **Do not** run `tmux kill-window`,
`git worktree remove`, or `git branch -d` yourself; the supervisor owns
that teardown.

### 5. Report to the caller

When a human is watching, tell them briefly: the branch merged into
`<source>`, how many commits, and that the worktree + window are being
cleaned up automatically. In autonomous/driver mode, there is nothing to
say — the `node report` IS the structured handoff the caller reads.

## Errors

Failures print a JSON envelope to **stderr** with a non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Always branch on `error.code`. On any failure the merge did NOT submit a
terminal report — the node stays live, so you can fix the cause and re-run
`run merge` safely.

Likely codes:

- `merge_failed` — the merge backend refused or the rebase hit conflicts.
  The message carries the backend's stderr. Common causes:
  - **Uncommitted changes** → `/git-commit` and re-run.
  - **Rebase conflicts** → resolve them, or for deeply diverged branches
    run `/complex-rebase`, then re-run `run merge`.
  - **Source branch not checked out in any worktree** (with `--source`) —
    the parent/integration worktree was removed; recreate it or correct
    `--source`.
  - **Lock timeout** — another merge held the cross-worktree lock past
    600s; retry.
- `run_not_found` — the run id (derived in step 1) names no run. Re-check
  the branch prefix; this may not be an orchestratectl-managed worktree.
- `no_worktree` / `no_branch` — the node has no worktree/branch recorded
  (a driver node, not a worker) — driver nodes are not merged.
- `schema_violation` / `report_file_invalid_json` /
  `report_file_too_large` — the `--report-file` payload is malformed.
  This is caught BEFORE the merge runs, so nothing happened — fix the file
  and re-run.

## Following up

After a successful merge the run is terminal:

- `orchestratectl run show <run-id>` — `status` reads `done` (or
  `failed`); the worktree/window are gone.
- `orchestratectl node show <run-id> n-0001` — the terminal report you
  submitted, with `via: "explicit-merge"`.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. On the first
invocation in a session, run `orchestratectl version --output json`, parse
the JSON, and read `.data.version`. Compare it to `{{CLI_VERSION}}`:

- **Missing**: install one of:
  - **Homebrew** (macOS/Linux): `brew install jarimustonen/orchestratectl/orchestratectl`
  - **Cargo** (any platform with a Rust toolchain): `cargo install orchestratectl`
  - **Shell installer** (no toolchain):
    `curl -LsSf https://github.com/jarimustonen/orchestratectl/releases/latest/download/orchestratectl-installer.sh | sh`

  (Publishing channels are TBD; the placeholders above mirror `issuectl`
  conventions and will be replaced once the release pipeline ships.)
- **Older than `{{CLI_VERSION}}`**: tell the user the skill expects
  `{{CLI_VERSION}}` and suggest upgrading via the same channel they
  originally used (`brew upgrade jarimustonen/orchestratectl/orchestratectl`,
  `cargo install orchestratectl --force`, or re-run the shell installer).
  Stop and wait — the `run merge` flag surface may have changed.
- **Newer than `{{CLI_VERSION}}`**: the installed binary is ahead of what
  this skill was written for. The whole bundled skill catalog has moved
  with the binary, so refresh all of them:
  `orchestratectl skill install --force` (add `--agent codex` for Codex or
  `--agent all` for both). To refresh only this skill, run
  `orchestratectl skill install worktree-merge --force`. Continue once the
  skills match.
- **Equal**: proceed normally.

## Examples

```
# Minimal: a spinoff worktree merges back to its recorded source with an
# auto-generated report.
short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
orchestratectl run merge "$run_id"

# Structured: a research worktree merges and delivers a §7.3 report.
orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json

# Fan-out unit: merges back into the shared source branch and reports.
orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json
```
