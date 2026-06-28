---
name: worktree-technical-decision
description: Spawn an autonomous worktree via `orchestratectl run create --kind technical-decision` to drive ONE architectural / technical decision to a recorded ADR and self-merge. Use when the user says "decide whether we should use X or Y", "make the architectural call on Z", "settle the trade-off between A and B", or links an issue tagged decision/architecture. Do NOT use for opinions (`/llm-consult`), design ideation (`/llm-workshop`), plan review (`/llm-panel`), survey/research (`/worktree-research`), or archaeology ("why did we choose X" — historical, not a forward decision).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-technical-decision

A **technical-decision worktree** is one autonomous agent whose
deliverable is a **recorded ADR** (architecture decision record) in the
repo — usually `docs/adr/<NNNN>-<slug>.md` or the project's equivalent
location. It investigates options, weighs trade-offs across required
expert lenses, picks one, records the decision with rationale and
explicit rejected alternatives, and merges itself back — same
self-merge contract as `worktree-spinoff`.

Read `orchestratectl-overview` first; read `worktree-spinoff` for the
shared autonomous-merge contract; read `worktree-research` for the
contrast — research surveys an open space, technical-decision picks
one path and records the call.

## When to use

- ✅ "Decide whether we should use X or Y".
- ✅ "Make the architectural call on Z".
- ✅ "Settle the trade-off between A and B".
- ✅ Issue tagged `decision` / `architecture` and the user says "drive
  this to an ADR".
- ❌ "What do you think of X" → `/llm-consult` (opinion, no record).
- ❌ "Design a system for X" → `/llm-workshop` (ideation, multi-LLM).
- ❌ "Review my plan" → `/llm-panel` (role panel, no merged ADR).
- ❌ "Compare A vs B vs C in depth" → `/worktree-research` (sourced
  report, no chosen path).
- ❌ "Why did we choose X" → archaeology; read past ADRs and the
  commit log, do not spawn anything.

## Workflow

### 0. Validate context

1. Working directory must be a git repo with a clean current branch.
2. ADR target directory must exist (typically `docs/adr/`). If it does
   not, ask the user where the ADR should land and create the
   directory in the worktree.
3. `orchestratectl version --output json` to confirm
   `{{CLI_VERSION}}`.

### 1. Pin the decision question

Decisions fail when the **question** drifts. Lock it down before
spawning:

- **Question** — one sentence, posed as a forward choice ("Should we
  use X or Y for Z?").
- **Constraints** — non-negotiable bounds (existing tech, deadlines,
  team skills, regulatory).
- **Options to consider** — at least two; the agent may add more if
  the space genuinely contains them but should not invent strawmen.
- **Expert lenses required** — typically architect + maintainability +
  security; add perf / cost / ergonomics as relevant.
- **Deliverable location** — ADR path.

If any of the above is missing, ask **once** before spawning.

### 2. Build the prompt

1. Pinned decision question + constraints.
2. Options to consider.
3. **Lens application** — agent runs the equivalent of `/llm-panel`
   over the question (architect, maintainability, security, plus
   topic-specific lenses) and synthesizes a recommendation.
4. **ADR structure** — Title / Status (Accepted) / Context / Decision
   / Consequences (including explicitly-rejected alternatives with
   reasons) / Date / Authors. Project-specific ADR templates take
   precedence if present.
5. **Done criteria** — ADR file exists at the agreed path, committed,
   merged back. No code changes unless the ADR mandates them (and
   even then, prefer a follow-up bugfix / code worktree to keep the
   ADR commit clean).

### 3. Create the run

```
orchestratectl run create \
  --kind technical-decision \
  --title "<adr-slug>" \
  --task "<self-contained decision brief>" \
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
    "kind": "technical-decision",
    "lifecycle": "autonomous",
    "tmux_window": "⚖️ wt/<adr-slug>",
    "branch": "wt/<adr-slug>"
  }
}
```

### 5. Report to the caller

Tell the user:

- Run id, branch, tmux window, expected ADR path.
- That the worktree self-merges once the ADR is committed.
- How to follow: `run show <run-id>`, `event tail --run <run-id>
  --follow`.

## Issue Management

If issue-driven (decision issue tagged `architecture`), the agent
links the merged ADR back to the issue and closes it on completion:

- `issuectl --json update <slug> --add-commit "<sha>:ADR <NNNN>"`
- `issuectl --json close <slug> --status done`

## Errors

Same envelope and codes as `worktree-spinoff`. One technical-decision
specific behavior: if the lens panel returns a genuine tie, the agent
does NOT pick randomly — it stops with `node report.success: false`
plus a `discuss[]` item naming the unresolved trade-off. The user
breaks the tie and re-spawns.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. Compare
`.data.version` from `orchestratectl version --output json` to
`{{CLI_VERSION}}`:

- **Missing**: install via Homebrew / Cargo / shell installer.
- **Older**: ask the user to upgrade; stop.
- **Newer**: `orchestratectl skill install --force` (or just
  `worktree-technical-decision --force`).
- **Equal**: proceed.

## Example

```
/worktree-technical-decision Choose between event-sourced and CRUD storage for the orchestratectl run state
```
