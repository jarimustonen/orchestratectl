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
- That the research worktree merges-and-reports itself via
  `orchestratectl run merge` — no `/worktree-merge` handoff.
- How to follow progress: `orchestratectl run show <run-id>` and
  `orchestratectl event tail <run-id> --follow`.

## Terminal report (mandatory)

Closing is **one call**. `orchestratectl run merge` owns the entire
merge-and-report step: it rebases + merges the worktree branch into its
source branch, then submits the terminal `node report` itself (stamped
`via: "explicit-merge"`). The run stays alive until that report lands —
until then the per-run supervisor keeps polling, `orchestratectl run
show` reads `lifecycle: pending`, and the tmux window never closes. So
the brief MUST instruct the agent to run the single closing call below
before its session ends. A research worktree's summary and wrap-up
matter, so it passes a `--report-file` carrying the full §7.3 payload
(the file is validated **before** the merge runs).

1. **Discover the run id** from inside the worktree. The branch is
   `wt/<short>-<slug>`, where `<short>` is the first 10 alphanumerics of
   the run id (the node id defaults to `n-0001` — a single-worker kind
   always has exactly one node):

   ```bash
   short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
   run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
   ```

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
   arrays empty — the call itself is what releases the supervisor.

3. **Merge and report in one call:**

   ```bash
   orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json
   ```

   This rebases + merges the worktree branch into its source branch and
   submits the §7.3 report in the same call. On a clean merge the per-run
   supervisor consumes the report, winds the run down, and tears down the
   worktree, tmux window, and branch automatically — do **not** manually
   run any tmux/git cleanup. If the source branch is not the run's
   recorded `source_branch`, pass `--source <branch>`.

   On a merge conflict the call exits non-zero with `error.code:
   "merge_failed"` and submits **no** report — the node stays live.
   Resolve the conflict (or run `/complex-rebase`) and re-run the same
   `run merge` call.

This step is **not optional**. No closing call leaves the run dangling,
with no structured outcome for the caller to read.

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
