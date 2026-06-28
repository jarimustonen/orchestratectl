---
name: worktree-spinoff
description: Spawn an autonomous spinoff worktree agent via `orchestratectl run create --kind spinoff` — one fire-and-forget agent that takes a focused task, executes it in its own git worktree, and merges itself back to the source branch. Use when the user says `/worktree-spinoff <task>`, when a parallel sub-task can be handled without interactive review, or when a driver (`/fan-out`, `/orchestrate`) needs to spawn one autonomous unit. NOT for interactive review (`/worktree-code`), N identical units (`/fan-out`), or dependency-ordered features (`/orchestrate`).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-spinoff

A **spinoff** is one autonomous agent running in its own git worktree,
doing one well-scoped task, and merging itself back to the source branch
when done. No interactive review. The canonical way to launch one is via
`orchestratectl`, which owns the run state under
`~/.orchestratectl/runs/<run-id>/` — never hand-craft branches or invoke
`workmux`/`create.sh` directly.

If you have not yet read it, read the `orchestratectl-overview` skill
first — it defines the run / supervisor / node vocabulary every step
below assumes.

## When to use

- ✅ User said `/worktree-spinoff <task>`.
- ✅ User asked to spawn a "background", "fire-and-forget", or
  "spinoff" worktree for a focused task.
- ✅ A driver skill (`/fan-out`, `/orchestrate`) needs to spawn one
  autonomous unit and pass `--parent-run-id` + `--parent-node-id`.
- ❌ User wants to review the diff interactively → `/worktree-code`.
- ❌ N≥5 similar independent units → `/fan-out`.
- ❌ Heterogeneous dependency-ordered features → `/orchestrate`.
- ❌ Substantial research / ADR / bugfix → use the matching
  `/worktree-research`, `/worktree-technical-decision`,
  `/worktree-bugfix` skill instead — they ship purpose-built prompt
  templates.

## Workflow

### 0. Validate context

1. If the working directory is not a git repo, abort with a clear
   message — the spinoff needs a source branch.
2. `orchestratectl version --output json` once per session. Compare
   `.data.version` to `{{CLI_VERSION}}` (see "Install or upgrade"
   below). Refuse to proceed on a major-version mismatch.
3. Capture the **current branch** with `git rev-parse --abbrev-ref HEAD`
   — it becomes the spinoff's source/merge target by default.

### 1. Identify task source

- **Issue-driven**: the user's prompt contains an issue reference
  (`#NN`, `issuectl:slug`, or a bare slug recognised by `issuectl
  --json show`). Read the issue via `issuectl --json show <ref>` and
  use its title + body as the task brief.
- **Freeform**: the user's prompt IS the task brief. Distill a 2–4 word
  title from it for `--title`.

Skip issue-driven detection when both `--parent-run-id` and
`--parent-node-id` are set (driver mode). An orchestrator fanning out
N spinoffs that all reference the same issue would otherwise update
and close that issue N times.

### 2. Build the prompt

The spinoff cannot ask follow-up questions. The `--task` string must be
self-contained. Include:

1. **Goal** — one sentence on what to deliver.
2. **Context** — files, modules, constraints. Quote relative paths.
3. **Done criteria** — concrete and verifiable (tests pass, no new
   clippy warnings, specific file exists).
4. **Quality bar** — does the spinoff need to run `/llm-review` before
   merging? Default is no review for spinoffs.
5. **Terminal report** — the brief MUST end with the mandatory terminal
   `node report` step (see "Terminal report (mandatory)" below). Without
   it the run never reaches `completed` and the worktree dangles.

If the prompt is longer than ~2 KB or contains characters that complicate
shell quoting, write it to a temp file and pass `--prompt-file
<path>` instead of `--task <string>`. Use `mktemp -t
spinoff-prompt-XXXXXX.md` and clean up after the call returns.

If any of Goal / Context / Done criteria is genuinely missing from the
user's request, ask the user **once** before spawning. A spinoff that
misinterprets the task wastes a worktree and a merge cycle.

### 3. Create the run

```
orchestratectl run create \
  --kind spinoff \
  --title "<2–4 word title>" \
  --task "<self-contained brief>" \
  [--source-branch <branch>] \
  [--headless | --tmux-session <name>] \
  [--parent-run-id <id> --parent-node-id <id>] \
  [--idempotency-key <key>]
```

Flag rules:

- `--kind spinoff` and `--title` are required.
- `--headless` places the agent's tmux window in a detached `headless`
  session instead of the foreground one, so a batch of spinoffs does not
  clutter the user's window list; attach later with `tmux attach -t
  headless`. `--tmux-session <name>` overrides the default session name
  (and implies headless). Auto-cleanup still closes the window on
  terminal. Example: `orchestratectl run create --kind spinoff
  --headless --title fix-lint --task "..."`.
- `--task` OR `--prompt-file` (exactly one). Empty/whitespace-only
  strings are rejected upstream — do not strip silently.
- `--source-branch` defaults to the current branch captured in step 0.
- `--parent-run-id` and `--parent-node-id` are mutually required; pass
  both or neither. Drivers (`/fan-out`, `/orchestrate`) pass them; a
  user-initiated `/worktree-spinoff` does not.
- `--idempotency-key` makes the call safe to retry on transient errors
  (network blip, disk full). Use the same key on retry and the CLI
  returns the original run without spawning twice.
- Output defaults to `--output jsonl` — one compact envelope per line.

### 4. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ...",
    "dir": "/Users/<you>/.orchestratectl/runs/01HZ...",
    "supervisor": 12345,
    "kind": "spinoff",
    "lifecycle": "autonomous",
    "node_id": "n-...",
    "tmux_window": "🚀 wt/<title>",
    "worktree_path": "/Users/<you>/.../worktrees/<title>",
    "branch": "wt/<title>"
  }
}
```

Read `data.run_id` — that is the handle for every follow-up
(`run show`, `node list`, `discussion list`). Read `data.supervisor` to
confirm the per-run supervisor process is alive; if it is `null` or the
field is `{"note": "..."}`, surface the note to the user and stop —
something blocked the supervisor spawn.

### 5. Report to the caller

Tell the user:

- Run id, kind (`spinoff`), source/merge branch.
- Tmux window name (so they can attach with `tmux attach -t
  octl <window>` if curious).
- That the spinoff merges itself — no `/worktree-merge` handoff from
  them.
- How to follow progress: `orchestratectl run show <run-id>` (or
  `--output jsonl` for one-line summaries).

When invoked from a driver, return the structured payload (run id, node
id, branch, tmux window) to the calling skill instead of a human
summary — the driver needs the IDs to poll completion.

## Terminal report (mandatory)

Merging is **not** the final step. The run stays alive until the agent
submits a terminal `node report`. Until that report lands the per-run
supervisor keeps polling, `orchestratectl run show` reads `lifecycle:
pending` forever, and the tmux window never closes — the user sees a
worktree that looks stuck when the work is actually done.

So the brief MUST instruct the spinoff to run this **immediately after
`/worktree-merge` succeeds, before its session ends**:

1. **Discover the run id and node id** from inside the worktree. The
   branch is `wt/<short>-<slug>`, where `<short>` is the first 10
   alphanumerics of the run id:

   ```bash
   short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
   run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
   node_id="n-0001"   # a single-worker kind always has exactly one node
   ```

2. **Write the §7.3 payload** to a temp file. These exact field names are
   what the supervisor consumes — do NOT use `discuss`,
   `spinoff_candidates`, or `wrap_up`: an unknown key still passes
   validation, but its contents are silently dropped.

   ```bash
   cat > /tmp/node-report.json <<'JSON'
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
   arrays empty — the call itself is what releases the supervisor.

3. **Submit it:**

   ```bash
   orchestratectl node report "$run_id" "$node_id" --from-file /tmp/node-report.json
   ```

   On success the node is recorded terminal — `orchestratectl node show
   <node-id>` reports `status: done` with your report attached.
   Submitting the report is the agent's **final action**: the per-run
   supervisor consumes it to wind the run down and (for autonomous
   kinds) close the worktree window. Do not wait for, re-verify, or
   re-submit it if `run show` still reads `pending` for a moment —
   supervisor-side completion on an agent-submitted report is still
   being wired up (issues `supervisor-complete-run-on-terminal-report`
   and `supervisor-close-tmux-on-terminal`); `orchestratectl run cancel
   <run-id>` is the documented manual cleanup until they land.

This step is **not optional**. A successful merge with no report leaves
the run dangling exactly as before, with no structured outcome for the
caller to read.

## Issue Management

Skip this section in driver mode (`--parent-run-id` set). The driver
owns issue interaction.

When issue-driven and not in driver mode, instruct the spinoff (via its
`--task` brief) to:

- Add commits as they happen:
  `issuectl --json update <NN> --add-commit "<sha>:<summary>"`
- Update status to in-progress on first commit:
  `issuectl --json update <NN> --status in-progress`
- Close on full completion:
  `issuectl --json close <NN> [--status fixed|done]`

The spinoff agent handles these calls itself; do not call `issuectl`
from this skill — it would race with the spinoff.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Always branch on `error.code`; the message is human prose.

Likely codes:

- `invalid_arguments` — missing/empty `--title` or `--task`, both
  `--task` and `--prompt-file` set, or `--parent-run-id` /
  `--parent-node-id` mismatched.
- `branch_not_found` — `--source-branch` does not exist locally. Fetch
  or correct the name; do not auto-create.
- `worktree_create_failed` — git refused (dirty working tree on the
  source branch, conflicting worktree path, locked branch). Report to
  the user; the source branch likely has uncommitted changes that must
  be committed or stashed first.
- `idempotent_replay` — informational; the `--idempotency-key` matched
  a prior run. The returned envelope describes that prior run; no new
  spawn happened.
- `supervisor_spawn_failed` — the supervisor process could not be
  started. The run dir exists but no one is driving the worker. Tell
  the user to inspect `<dir>/supervisor.stderr.log` and consider
  `orchestratectl run reattach <run-id>`.

If `--dry-run` is set, the CLI validates inputs and emits a
`dry_run: true` envelope without materializing anything.

## Following progress

The spinoff runs asynchronously. To check status:

- `orchestratectl run show <run-id>` — current lifecycle, node states,
  recent events.
- `orchestratectl event tail <run-id> --follow` — streaming
  event log (use for "wait until merged" loops).
- `orchestratectl node list <run-id>` — per-unit detail (a
  spinoff has exactly one worker node).
- `orchestratectl node show <node-id>` — the structured terminal
  report the spinoff submits when it merges (the `node report` verb is
  for *writing* that report; see "Terminal report (mandatory)").

`lifecycle` is the only field that tells you the run is finished:
`completed` (worker merged), `failed` (worker errored), `cancelled`
(`run cancel` was called). The `status` field is a short human label —
do not branch on its text.

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
  `orchestratectl skill install worktree-spinoff --force`. Continue
  once the skills match.
- **Equal**: proceed normally.

## Examples

```
# Freeform spinoff
/worktree-spinoff Process receipts batch 2026-05 with vision OCR

# Issue-driven (skill reads issue NN, builds task brief from it)
/worktree-spinoff #142

# Driver mode — only /fan-out and /orchestrate pass these
orchestratectl run create --kind spinoff \
  --title "u-003-receipts" \
  --task "..." \
  --source-branch fan-out/2026-05 \
  --parent-run-id 01HZ... \
  --parent-node-id n-0001
```
