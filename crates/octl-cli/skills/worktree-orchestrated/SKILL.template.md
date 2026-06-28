---
name: worktree-orchestrated
description: Spawn ONE autonomous worktree agent via `orchestratectl run create --kind orchestrated --parent-run-id <id> --parent-node-id <id>` as a child unit of an `/orchestrate` DAG. The child implements one feature with its own best-judgment decisions, reviews itself, merges into a shared integration branch, and submits a structured terminal `node report` (success, discuss items + chosen path, spin-off candidates, wrap-up recommendations) the parent supervisor consumes. NOT a user-facing slash command — invoked by `/orchestrate`. For identical independent units use `/worktree-spinoff`; for interactive human-reviewed work use `/worktree-code`.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-orchestrated

An **orchestrated worktree** is one autonomous agent spawned as a child
of a parent `/orchestrate` run. It implements one feature, merges itself
into a shared integration branch, and submits a structured terminal
`node report` the parent reads from its event log. This skill is the
adapter `/orchestrate` uses to launch one such child via the canonical
`orchestratectl run create --kind orchestrated` call — not something a
human user invokes directly.

If you have not yet read it, read the `orchestratectl-overview` skill
first — every term below (run, node, supervisor, child-spawn, report)
is defined there. Read `worktree-spinoff` for the contract template
this skill mirrors.

## When to use

- ✅ `/orchestrate` is fanning out the next ready feature in its DAG
  and needs one autonomous child unit with a known parent run/node.
- ❌ Any human-facing invocation. If a user types
  `/worktree-orchestrated` directly, redirect them to `/worktree-code`
  (interactive) or `/worktree-spinoff` (autonomous, no parent).

## Workflow

### 0. Validate driver context

The caller MUST be a driver — refuse otherwise.

1. Both `--parent-run-id` and `--parent-node-id` must be supplied; this
   is a hard precondition. Without them the call is not orchestrated
   and the wrong skill is being invoked.
2. The caller must have already chosen the target **integration
   branch** (the shared branch every sibling unit merges into) and
   ensured it exists. Capture it as the `--source-branch` for this
   child.
3. `orchestratectl version --output json` once per session to confirm
   `{{CLI_VERSION}}` matches the running binary.

### 1. Build the child's prompt

The child cannot ask the parent follow-up questions. The brief must be
self-contained and explicit about:

1. **Feature goal** — one paragraph; the slice of the DAG this child
   owns.
2. **Inputs / constraints** — files, modules, contracts the upstream
   siblings agreed on.
3. **Done criteria** — concrete and verifiable (tests, types, specific
   files exist).
4. **Self-review expectation** — the child runs `/llm-review` (or
   `/llm-panel` for design/decision artefacts) and `/assess-findings`
   on its own diff and applies "fix now" items autonomously.
5. **Structured terminal report** — when the child finishes, it submits
   a `node report` envelope with:
   - `success: true|false`
   - `discuss[]` — items that needed a human call; for each, record
     `chosen_path` and `severity: info|warn|critical`
   - `spinoff_candidates[]` — follow-up work proposals (one-line
     summary + suggested kind)
   - `wrap_up[]` — recommendations for the orchestrator (further
     reviews, additional siblings, doc updates)
6. **Merge target** — the integration branch, supplied via
   `--source-branch`. The child's `--kind orchestrated` recipe merges
   itself into this branch on success.

Long prompts → temp file + `--prompt-file <path>` instead of `--task`.

### 2. Create the child run

```
orchestratectl run create \
  --kind orchestrated \
  --title "<unit slug, e.g. f-003-receipts>" \
  --task "<self-contained child brief>" \
  --source-branch <integration-branch> \
  --parent-run-id <parent-run-id> \
  --parent-node-id <parent-node-id> \
  [--idempotency-key <key>]
```

Flag rules:

- `--kind orchestrated`, `--title`, `--task`/`--prompt-file`,
  `--source-branch`, `--parent-run-id`, `--parent-node-id` are all
  required. The parent pair is mutually required and rejected if only
  one is set.
- `--idempotency-key` is strongly recommended in DAG context: the
  parent may retry on transient errors and the same key returns the
  original child run instead of spawning twice.
- The CLI emits `child.spawned` on the **parent's** event log first
  (single-arbiter invariant), then initializes the child's run dir and
  shells out to `create.sh`. The parent's supervisor sees
  `child.spawned` and is the sole spawner of the child's supervisor —
  the driver does NOT call `orchestratectl supervise` on the child.
- Output defaults to `--output jsonl`.

### 3. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ-CHILD",
    "dir": "/Users/<you>/.orchestratectl/runs/01HZ-CHILD",
    "supervisor": {"note": "child supervisor spawned by parent"},
    "kind": "orchestrated",
    "lifecycle": "autonomous",
    "parent_run_id": "01HZ-PARENT",
    "parent_node_id": "n-driver-001",
    "node_id": "n-...",
    "tmux_window": "🎼 wt/<title>",
    "worktree_path": "/Users/<you>/.../worktrees/<title>",
    "branch": "wt/<title>"
  }
}
```

The `supervisor` field is intentionally a `{"note": "..."}` here — the
parent's supervisor spawns the child's. Do NOT treat the note as an
error.

Return the structured payload (run id, node id, branch, parent
pointers) to the calling `/orchestrate` driver. The driver polls the
child's lifecycle and `node report` via `orchestratectl run show <id>`
and `orchestratectl event tail --run <parent-run-id> --follow`.

### 4. Report to the driver

`/orchestrate` is the only caller. Hand back:

- Child run id, child node id.
- Branch + tmux window (so the driver can attach if a human asks).
- Integration branch (where the child will merge).
- Reminder that the parent supervisor — not the driver — will spawn
  the child's supervisor.

## Issue Management

Orchestrated children do NOT touch `issuectl`. The parent
`/orchestrate` driver owns issue interaction so that N children
referencing the same epic do not race to update or close it.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Likely codes:

- `invalid_arguments` — `--parent-run-id` and `--parent-node-id` not
  both set, missing/empty `--title` or `--task`, or unknown flag.
- `parent_run_not_found` — the parent run id does not exist on disk.
- `parent_node_not_found` — the parent node id does not exist in the
  parent run's manifest.
- `child_spawn_failed` — `child.spawned` could not be appended to the
  parent's event log (parent log corrupt, lock contention). Retry
  with the same `--idempotency-key`.
- `worktree_create_failed` — git refused for the integration branch
  (dirty tree, conflicting worktree path).
- `idempotent_replay` — informational; key matched a prior child.
- `dry_run_unsupported` — child-spawn cannot be truthfully dry-run.

## Following progress

The child writes its events to **its own** run log, and the parent's
supervisor mirrors lifecycle transitions onto the parent's log so a
driver only needs to tail the parent:

- `orchestratectl event tail --run <parent-run-id> --follow` —
  authoritative stream for the driver; `child.spawned`,
  `child.lifecycle`, and `child.report` events arrive here.
- `orchestratectl run show <child-run-id>` — child-only detail.
- `orchestratectl node report <child-node-id>` — the structured
  terminal report the child submits at the end. This is what the
  orchestrator synthesizes across siblings.

`lifecycle: completed` on the child means the child merged into the
integration branch and submitted its report. The driver should not
treat `success: false` inside the report as a CLI error — it is a
truthful structured outcome the orchestrator must surface to the user.

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

  (Publishing channels are TBD.)
- **Older than `{{CLI_VERSION}}`**: tell the user the skill expects
  `{{CLI_VERSION}}` and suggest upgrading. Stop — child-spawn semantics
  may have changed.
- **Newer than `{{CLI_VERSION}}`**: refresh the catalog:
  `orchestratectl skill install --force` (or `worktree-orchestrated`
  alone with `--force`).
- **Equal**: proceed normally.

## Example

```
# Driver-only invocation: /orchestrate calls this skill once per
# DAG-ready feature with the parent pointers in hand.
orchestratectl run create \
  --kind orchestrated \
  --title "f-003-receipts" \
  --task "@/tmp/f-003-receipts-brief.md" \
  --source-branch orchestrate/integration-2026-06-28 \
  --parent-run-id 01HZ-PARENT \
  --parent-node-id n-driver-001 \
  --idempotency-key f-003-receipts-v1
```
