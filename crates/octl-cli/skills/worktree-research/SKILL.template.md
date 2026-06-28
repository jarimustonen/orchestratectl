---
name: worktree-research
description: Spawn an autonomous research worktree via `orchestratectl run create --kind research` — multi-source background investigation that reads sources, gathers divergent perspectives, and writes a sourced markdown report committed to the repo, then merges itself back. Use when the user asks to research, investigate, survey, look into, or compare options for a topic that needs reading multiple sources and synthesizing into a sourced markdown report. Do NOT use for quick factual lookups (one `WebSearch`), single-doc summaries, debugging, code changes, or forward decisions (`/worktree-technical-decision`).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# worktree-research

A **research worktree** is one autonomous agent whose deliverable is a
**sourced markdown report**, not code. It reads multiple sources, weighs
divergent perspectives, writes the report into the repo (typically
`research/<slug>.md`), commits, and merges itself back to the source
branch — same self-merge contract as `worktree-spinoff` but with a
prose-output recipe and `WebSearch`/`WebFetch` enabled.

Read `orchestratectl-overview` first; read `worktree-spinoff` for the
shared autonomous-merge contract.

## When to use

- ✅ "Research the state of X", "investigate options for Y", "survey
  approaches to Z", "compare A vs B vs C in depth".
- ✅ The deliverable is a written report the user will read and cite
  later.
- ❌ One-shot fact lookup (just `WebSearch`).
- ❌ Single-doc / single-file summary (read it inline).
- ❌ Debugging or code changes → `/worktree-bugfix`, `/worktree-code`.
- ❌ "Which option should we pick?" — that is a decision, use
  `/worktree-technical-decision` (it records an ADR).

## Workflow

### 0. Validate context

1. Working directory must be a git repo. Per repo CLAUDE.md, the
   current branch must be clean.
2. `orchestratectl version --output json` to confirm
   `{{CLI_VERSION}}` matches the running binary.
3. Capture the current branch as the source/merge target.

### 1. Sharpen the question

Research outputs are only as good as their question. Before spawning,
distill the user's request to:

- **Core question** — one sentence.
- **Scope** — what counts as in-bounds (timeframe, ecosystems,
  platforms, audience).
- **Out-of-scope** — explicit exclusions; prevents the agent from
  wandering.
- **Audience + tone** — engineer-facing? executive-facing? Finnish?
- **Output location** — default `research/<slug>.md`; override if the
  user has a different repo convention.

If any of the above is genuinely missing from the user's prompt, ask
**once** before spawning. A misdirected research run wastes hours.

### 2. Build the prompt

Include in the brief:

1. The four-part framing from step 1 (question / scope / exclusions /
   audience).
2. **Source-quality bar** — prefer primary sources (RFCs, vendor
   docs, original papers) over aggregators; cite every non-trivial
   claim with a URL.
3. **Divergent perspectives** — explicitly require N≥2 distinct
   viewpoints when the topic admits them; do not present a single
   narrative as consensus.
4. **Report structure** — TL;DR (3–5 bullets) → Background → Options /
   Findings → Trade-offs → Citations. Keep claims and citations
   inline.
5. **Done criteria** — file at `research/<slug>.md` exists, committed,
   merged back to source branch.

Long prompts → temp file + `--prompt-file <path>`.

### 3. Create the run

```
orchestratectl run create \
  --kind research \
  --title "<2–4 word slug>" \
  --task "<self-contained research brief>" \
  [--source-branch <branch>] \
  [--idempotency-key <key>]
```

Same flag rules as `worktree-spinoff` — `--kind research`,
`--title`, and `--task`/`--prompt-file` required; `--source-branch`
defaults to the current branch. Output defaults to `--output jsonl`.

### 4. Success envelope

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ...",
    "supervisor": 12345,
    "kind": "research",
    "lifecycle": "autonomous",
    "node_id": "n-...",
    "tmux_window": "🔬 wt/<title>",
    "worktree_path": "/Users/<you>/.../worktrees/<title>",
    "branch": "wt/<title>"
  }
}
```

Read `data.run_id` and `data.supervisor`. If supervisor is `null` or
`{"note": "..."}`, surface and stop.

### 5. Report to the caller

Tell the user:

- Run id, branch, tmux window.
- Expected output path: `research/<slug>.md` (or the override they
  specified).
- That the research worktree merges itself — no `/worktree-merge`
  handoff.
- How to follow progress: `orchestratectl run show <run-id>` and
  `orchestratectl event tail --run <run-id> --follow`.

## Issue Management

Skip when driver-spawned. When issue-driven and standalone, instruct
the research agent to record the produced report path on the issue:

- `issuectl --json update <slug> --add-commit "<sha>:research report"`
- `issuectl --json close <slug> --status done` only if the issue is
  literally "produce this research", not if research is one step in a
  larger feature.

## Errors

Same envelope and codes as `worktree-spinoff` (`invalid_arguments`,
`branch_not_found`, `worktree_create_failed`, `idempotent_replay`,
`supervisor_spawn_failed`). One research-specific gotcha: if
`WebSearch`/`WebFetch` is disabled in the worktree's tool allowlist,
the run will be useless — the project's CLAUDE.md / `.workmux.yaml`
must expose those tools to `--kind research`. The CLI does not
currently validate this; surface it to the user if the research output
comes back with "could not fetch sources" notes.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. On the
first invocation in a session, run
`orchestratectl version --output json`, compare `.data.version` to
`{{CLI_VERSION}}`, and:

- **Missing**: install via Homebrew / Cargo / shell installer (see the
  contract-template skills like `worktree-spinoff` for channels).
- **Older**: tell the user to upgrade and stop.
- **Newer**: `orchestratectl skill install --force` (or just
  `worktree-research --force`).
- **Equal**: proceed.

## Example

```
/worktree-research Compare WAL implementations in SQLite, DuckDB, and Postgres for write-heavy embedded use
```
