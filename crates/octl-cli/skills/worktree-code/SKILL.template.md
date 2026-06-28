---
name: worktree-code
description: Spawn an INTERACTIVE coding agent in its own git worktree via `orchestratectl run create --kind code` — human-reviewed parallel work where the user finalizes the merge with `/worktree-merge`. Use when the user says `/worktree-code <task>`, `/worktree <task>` (router picks this for review-oriented work), invokes with an `issuectl` slug, or asks to "spawn a worktree to do X" without saying "spinoff", "research", or another variant. NOT for autonomous fire-and-forget (`/worktree-spinoff`), N identical units (`/fan-out`), dependency-ordered features (`/orchestrate`), research (`/worktree-research`), ADRs (`/worktree-technical-decision`), or skill authoring (`/worktree-make-skill`).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-code

An **interactive worktree** is one coding agent running in its own git
worktree, expected to pause for human review and hand the merge back to
the user via `/worktree-merge`. This is the workhorse for any non-trivial
change the user wants to eyeball before it lands. The canonical way to
launch one is via `orchestratectl`, which owns the run state under
`~/.orchestratectl/runs/<run-id>/`.

If you have not yet read it, read the `orchestratectl-overview` skill
first — it defines the run / supervisor / node vocabulary every step
below assumes. Read `worktree-spinoff` if you want the autonomous
counterpart.

## When to use

- ✅ User said `/worktree-code <task>` or `/worktree <task>` (the
  router picks this for review-oriented work).
- ✅ User passed an `issuectl` issue slug (`/worktree
  extremely-quiet-otter`) and wants to work on it interactively.
- ✅ User said "spawn a worktree for X" / "do this in a worktree" with
  no hint that it should be autonomous.
- ❌ User said "fire-and-forget", "spinoff", or "background" →
  `/worktree-spinoff`.
- ❌ N≥5 similar independent units → `/fan-out`.
- ❌ Heterogeneous dependency-ordered features → `/orchestrate`.
- ❌ Research / ADR / bugfix / skill authoring → the matching
  `/worktree-*` skill.

## Workflow

### 0. Validate context

1. If the working directory is not a git repo, abort with a clear
   message — the worktree needs a source branch.
2. `orchestratectl version --output json` once per session. Compare
   `.data.version` to `{{CLI_VERSION}}` (see "Install or upgrade"
   below). Refuse to proceed on a major-version mismatch.
3. Capture the **current branch** with `git rev-parse --abbrev-ref HEAD`
   — it becomes the worktree's source branch and the merge target for
   `/worktree-merge`.
4. Per the repo CLAUDE.md, `main` must be clean (no uncommitted
   changes) before spawning. If `git status --porcelain` returns
   non-empty on the current branch, stop and tell the user — they
   should commit or stash first so the worktree forks from the
   intended state.

### 1. Identify task source

- **Issue-driven**: the user's prompt contains an issue reference
  (`#NN`, a bare slug recognised by `issuectl --json show`, or
  `@<slug>`). Read the issue via `issuectl --json show <ref>` and use
  its title + body + sibling files (`plan.md`, `analysis.md`,
  `todo.md`) as the task brief.
- **Freeform**: the user's prompt IS the task brief. Distill a 2–4 word
  title from it for `--title`.

If neither a task nor an issue reference is present, ask the user
**once** for clarification before proceeding.

### 2. Build the prompt

The agent will pause for human review at the merge point, but it cannot
re-ask the user about the original brief. The prompt should include:

1. **Objective** — what to deliver.
2. **Context** — files, modules, constraints. Quote relative paths.
   When issue-driven, populate from the issue body and sibling files
   instead of inventing context.
3. **Files to examine** — relative paths only.
4. **Success criteria** — derive from the issue's Acceptance Criteria /
   Quick Test, or restate the freeform brief explicitly.
5. **Issue management** (issue-driven only) — instruct the agent to
   record commits and status via `issuectl --json update <slug>
   --add-commit "..." --status in-progress` and to close on
   completion.
6. **Review + merge handoff** — autonomous up to the merge: the agent
   commits via `/git-commit`, runs `/llm-review` (or `/llm-panel` for
   design/decision artefacts), runs `/assess-findings`, applies "fix
   now" rows, decides autonomously whether a second review round is
   needed, and finally runs `/wrap-up` (which IS user-interactive — it
   waits for the user to confirm before saving session context). After
   `/wrap-up`, the user runs `/worktree-merge` themselves. The terminal
   `node report` that moves the run to `completed` is the post-merge
   closeout — see "Terminal report (mandatory)" below.

For long or special-character-heavy prompts, write them to a temp file
(`mktemp -t worktree-code-prompt-XXXXXX.md`) and pass `--prompt-file
<path>` instead of `--task <string>`.

### 3. Create the run

```
orchestratectl run create \
  --kind code \
  --title "<2–4 word title, or issue slug>" \
  --task "<self-contained brief>" \
  [--source-branch <branch>] \
  [--layout <name>] [--no-hooks] \
  [--idempotency-key <key>]
```

Flag rules:

- `--kind code` and `--title` are required.
- `--task` OR `--prompt-file` (exactly one). Empty/whitespace-only
  strings are rejected upstream — do not strip silently.
- `--source-branch` defaults to the current branch captured in step 0.
- `--layout <name>` selects a named layout from the project's
  `.workmux.yaml` (e.g. `with-test-server` for cargo-watch + pnpm +
  gsdev). Default layout is lightweight (agent pane + plain shell);
  opt into a heavier layout only when the work needs a running server.
- `--no-hooks` skips the workmux `post_create` hooks. Combinable with
  any layout. Use sparingly — hooks usually do useful setup.
- `--idempotency-key <key>` makes the call safe to retry on transient
  errors (network blip, disk full). Use the same key on retry and the
  CLI returns the original run without spawning twice.
- `--parent-run-id` / `--parent-node-id` are NOT valid here — interactive
  worktrees are top-level only. If the caller is a driver wanting a
  child unit, it must use a different `--kind`.
- Output defaults to `--output jsonl` — one compact envelope per line.

### 4. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ...",
    "dir": "/Users/<you>/.orchestratectl/runs/01HZ...",
    "supervisor": 12345,
    "kind": "code",
    "lifecycle": "interactive",
    "node_id": "n-...",
    "tmux_window": "🎬 wt/<title>",
    "worktree_path": "/Users/<you>/.../worktrees/<title>",
    "branch": "wt/<title>"
  }
}
```

`lifecycle: interactive` is the load-bearing difference from a spinoff:
the supervisor will NOT auto-merge the branch; the user owns the merge
via `/worktree-merge` after reviewing. Read `data.run_id` and
`data.supervisor` — if the supervisor field is `null` or `{"note":
"..."}`, surface it to the user and stop.

### 5. Report to the caller

Tell the user:

- Run id, branch name, tmux window.
- Source/merge branch (where `/worktree-merge` will land it).
- That the worktree is interactive — they review when ready, then run
  `/worktree-merge`. Do NOT promise auto-merge; that would mislead.
- How to attach: `tmux attach -t octl <window>`.
- How to follow progress: `orchestratectl run show <run-id>`.

## Terminal report (mandatory)

A `code` run is **interactive**: the agent works autonomously up to
`/wrap-up`, then the human reviews and runs `/worktree-merge`. But the
run is not finished until a terminal `node report` lands — without it the
supervisor keeps polling and `orchestratectl run show` reads `lifecycle:
pending` forever, the same dangling-worktree symptom autonomous kinds
hit. The difference is only *when* the report fires: it is the
**post-merge closeout**, run once the human's `/worktree-merge` has
landed and the worktree is done being used — NOT during the agent's
session (a terminal report mid-session would try to close the window the
user is still working in).

Submit it once the merge is complete:

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
     cleanly; `false` when reporting a blocked or abandoned outcome.
   - `summary` — optional one-line human-readable result.
   - `discussion_items[]` — decisions that needed a human call. Each:
     `{"topic": "<non-empty>", "severity": "discuss|critical|info",
     "options": ["…"]}`.
   - `spinoff_proposals[]` — follow-up work worth spawning. Each:
     `{"proposed_title": "<non-empty>", "proposed_kind":
     "spinoff|code|research|bugfix|technical-decision|make-skill|fan-out|orchestrated",
     "rationale": "<why>"}`.
   - `wrap_up_recommendations[]` — array of strings; advice for the
     caller (further reviews, doc updates, follow-up work).

3. **Submit it:**

   ```bash
   orchestratectl node report "$run_id" "$node_id" --from-file /tmp/node-report.json
   ```

   On success the node is recorded terminal — `orchestratectl node show
   <node-id>` reports `status: done` with the report attached. The
   per-run supervisor consumes it to wind the interactive run down. Do
   not re-submit if `run show` briefly still reads `pending` —
   supervisor-side completion on an agent-submitted report is still being
   wired up (issues `supervisor-complete-run-on-terminal-report` and
   `supervisor-close-tmux-on-terminal`); `orchestratectl run cancel
   <run-id>` is the documented manual cleanup until they land.

## Issue Management

When issue-driven, the prompt instructs the agent to:

- Add commits as they happen:
  `issuectl --json update <slug> --add-commit "<sha>:<summary>"`
  (or rely on `Refs-Issue: <slug>` / `Fixes-Issue: <slug>` commit
  trailers plus `issuectl sync-commits`).
- Update status to in-progress on first commit:
  `issuectl --json update <slug> --status in-progress`.
- Close on full completion:
  `issuectl --json close <slug> [--status fixed|done]`.

The agent owns these calls; do not call `issuectl` from this skill — it
would race with the agent.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Always branch on `error.code`; the message is human prose.

Likely codes:

- `invalid_arguments` — missing/empty `--title` or `--task`, both
  `--task` and `--prompt-file` set, or unknown flag.
- `branch_not_found` — `--source-branch` does not exist locally.
- `worktree_create_failed` — git refused (dirty working tree on the
  source branch, conflicting worktree path, locked branch). The source
  branch likely has uncommitted changes that must be committed or
  stashed first.
- `idempotent_replay` — informational; the `--idempotency-key` matched
  a prior run.
- `supervisor_spawn_failed` — the supervisor process could not be
  started. Inspect `<dir>/supervisor.stderr.log` and consider
  `orchestratectl run reattach <run-id>`.

If `--dry-run` is set, the CLI validates inputs and emits a
`dry_run: true` envelope without materializing anything.

## Following progress

Interactive runs idle until the user attaches and drives the agent.
While the supervisor watches:

- `orchestratectl run show <run-id>` — lifecycle, node states, recent
  events.
- `orchestratectl event tail --run <run-id> --follow` — streaming event
  log.
- `orchestratectl discussion list --run <run-id>` — decisions the
  agent surfaced that need human input. Resolve via `discussion
  resolve` before the run can continue past them.
- `orchestratectl spinoff list --run <run-id>` — spin-off proposals
  the agent raised during review. The user approves/rejects with
  `spinoff approve` / `spinoff reject`.

`lifecycle` is the only field that tells you the run is finished:
`completed` (user merged), `failed` (agent errored), `cancelled` (`run
cancel` was called).

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
  originally used. Stop and wait — the `run create --kind code` flag
  surface may have changed.
- **Newer than `{{CLI_VERSION}}`**: the installed binary is ahead of
  what this skill was written for. Refresh the whole catalog:
  `orchestratectl skill install --force` (add `--agent codex` for Codex
  or `--agent all` for both). To refresh only this skill, run
  `orchestratectl skill install worktree-code --force`. Continue once
  the skills match.
- **Equal**: proceed normally.

## Examples

```
# Freeform
/worktree-code Add /help command

# Issue-driven (skill reads the issue and builds the brief from it)
/worktree-code extremely-quiet-otter
/worktree-code #142

# Heavier layout for full dev stack
/worktree-code --with-test-server Migrate ops::receipts to new tracing macros
```

(`--with-test-server` is a thin alias the slash-command layer resolves
to `--layout with-test-server` before calling `orchestratectl run
create`.)
