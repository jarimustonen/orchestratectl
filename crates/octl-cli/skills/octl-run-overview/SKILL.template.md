---
name: octl-run-overview
description: Read the output of `orchestratectl run list` and `orchestratectl run show` to inspect the state of orchestrated agent workflows (worktrees, fan-outs, spinoffs). Use when asked about run status, when triaging an in-flight orchestration, or before deciding whether to spawn, resume, or abort work.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# octl-run-overview

> ## ⚠️ PREVIEW — DO NOT INVOKE BLINDLY
>
> The `orchestratectl run list` / `orchestratectl run show` subcommands
> documented here are **not yet implemented** in this build. They land in
> a follow-up issue (`run-list-show`). Until then, this file is a forward
> contract: read it to understand the shape of the response, but **call
> `orchestratectl version` first** and refuse to execute `run` commands
> unless they appear in `--help`. If they don't, tell the user the
> feature is not yet shipped and stop.

`orchestratectl` is the state owner for agent workflows: worktrees,
fan-outs, orchestrations, and spinoffs. Every workflow is a **run** with
canonical state under `~/.orchestratectl/runs/<run-id>/`. Two commands
will expose that state to you once they ship:

- `orchestratectl run list` — every run, newest first
- `orchestratectl run show <run-id>` — one run, full detail (one-shot)
- `orchestratectl run wait <run-id> …` — the blocking counterpart to
  `run show`: poll one or more runs with sane backoff until they reach a
  terminal state (`done | failed | cancelled`), then emit a structured
  summary. Use this instead of hand-rolling a `while … run show … case`
  loop (`--any` returns on the first terminal run; `--timeout <dur>` and
  `--fail-on-error` shape the exit code).

The default output is `--output jsonl` — one compact JSON envelope per
line on stdout. Agents read this directly with `serde_json::from_str`
on each line. Pass `--output text` only when a human is reading the
terminal; pass `--output json` if you want pretty-printed JSON instead
of the single-line stream. The `--json` flag from older builds is
gone — `--output jsonl` is the canonical machine form.

## Envelope

Every success looks like:

```json
{
  "schema_version": 1,
  "data": { ... },
  "warnings": ["..."]
}
```

`schema_version` is the envelope version. If you see a number you do not
recognise, refuse to proceed and report the mismatch — the state shape
may have changed under you. `warnings` is optional; surface it to the
user when present.

## `run list` payload

`data.runs` is an array of summary objects. Sort order is newest-first
by `created_at` (RFC3339).

```json
{
  "data": {
    "runs": [
      {
        "id": "01HZ...",
        "kind": "spinoff | research | technical-decision | fan-out",
        "lifecycle": "autonomous | interactive",
        "status": "pending | running | done | failed | cancelled",
        "created_at": "2026-06-12T10:30:00Z",
        "updated_at": "2026-06-12T10:45:12Z"
      }
    ]
  }
}
```

Fields that drive decisions:

- `kind` — the run's **topology**; picks the right follow-up command (a
  `fan-out` resumes differently from a single `spinoff`).
- `lifecycle` — the run's **how-run category**, set explicitly at
  `run create` from the `--interactive` flag (NOT derived from `kind`).
  `autonomous` (the default — fire-and-forget; the supervisor adjudicates
  exit and tears down) or `interactive` (human-driven — the supervisor
  never auto-terminalizes; it waits for an explicit `run merge` /
  `run cancel`, so an interactive run can sit non-terminal indefinitely by
  design). It is NOT a progress state and never transitions. Read it to
  know *how* a run is driven; read `status` for whether it is *done*.
- `status` — the **terminal-progress field**. Values are `pending`,
  `running`, `done`, `failed`, `cancelled`. **Terminal states are
  `done | failed | cancelled`** — once any of those is set the run is
  settled (the reducer freezes further status changes). Branch on this
  to detect completion.

## `run show` payload

`run show`'s `data` carries the **same flat row a `run list` row does**
at the top level (`run_id`, `kind`, `status`, `title`, `created_at`,
`node_count`, `supervisor`, `stalled`) — so you can address these the
same way across both verbs. `data.manifest` then extends that row with
full detail (`lifecycle`, `updated_at`, `source_*`, `parent_*`,
`open_discussions`, `pending_spinoffs`); `data.counts` carries
denormalised counters; `data.supervisor` is the probed supervisor
liveness; `landed`/`landed_method`/`recoverable_work`/`false_failed` are
`run show`-only computed detail; some kinds add kind-specific fields.

`data.false_failed` (present only when set) flags a **suspected
false-failed run**: the run is `failed` yet git confirms the worker's
content is already in source (`landed: true`, `landed_method:
"git-verified"`) with no `run merge` on record — the raw-git
self-merge-then-death case. It is a **non-mutating hint, never an
auto-success**: the run stays `failed`. Its `resume_hint` steers you to
`orchestratectl run salvage <id>`, which records the skipped merge
through the real `run merge` machinery (idempotent against the
already-integrated content) and terminalizes the run to `done` honestly.
Do NOT treat a `false_failed` run as done — run salvage first. Never
finish a run with a raw `git merge`; always use `run merge`/`run
salvage`.

`data.supervisor.state` is the field to branch on — it distinguishes the
conditions the legacy `alive` boolean collapses: `alive` (running),
`dead` (started then died / recycled — orphaned, recover with `run
reattach`), `not-recorded` (never launched or cleanly torn down),
`unreadable` (pid file present but can't be parsed — investigate), and
`unknown` (not probed; you won't see it on `run show`/`run list`, which
always probe). `data.supervisor.alive` is retained for back-compat and
equals `state == "alive"` — prefer `state`, since only it tells
"orphaned" from "finished" from "I/O error".

```json
{
  "data": {
    "run_id": "01HZ...",
    "kind": "fan-out",
    "status": "running",
    "title": "...",
    "created_at": "...",
    "node_count": 10,
    "supervisor": { "pid": 65745, "state": "alive", "alive": true },
    "stalled": false,
    "manifest": {
      "schema_version": 1,
      "run_id": "01HZ...",
      "kind": "fan-out",
      "lifecycle": "autonomous",
      "title": "...",
      "status": "running",
      "created_at": "...",
      "updated_at": "...",
      "node_count": 10,
      "open_discussions": 0,
      "pending_spinoffs": 0
    },
    "counts": { "nodes": 10, "discussions": 0, "spinoffs": 0 }
  }
}
```

Both `data.status` (flat) and `data.manifest.status` resolve to the same
value; the flat path matches `run list`, the nested one is kept for
back-compat.

## Decision rules

1. **Triaging "is this still going?"** — read `data.manifest.status`.
   Terminal values (`done | failed | cancelled`) mean the run is settled
   and the supervisor has either already torn it down or is about to.
   Anything else (`pending | running`) is still live.
2. **Polling "wait until merged"** — loop on
   `orchestratectl run show <id> --output json | jq -r '.data.manifest.status'`
   and break on `done|failed|cancelled`. Do NOT poll `lifecycle` — it is
   the category, not a progress field, and never transitions.
3. **Deciding whether to spawn more work** — list runs first. If a
   `fan-out` is already `running` on the same scope, do not start a
   second; resume or wait.
4. **Surfacing problems to the user** — when `status == "failed"`, the
   event log has the cause; quote it instead of guessing.
5. **Schema drift** — if `schema_version` does not match what this skill
   describes, stop and tell the user the skill is out of date with the
   installed binary.
6. **Unblocking ONE stuck fan-out child** — cancel a whole run with
   `orchestratectl run cancel <run-id>`; cancel a single live node with
   `orchestratectl run cancel <run-id> --node <node-id>`. The per-node form
   is branch-preserving (source-relative teardown — a child's committed
   work is never force-deleted) and does NOT terminalize the run while
   other nodes are still live: the supervisor rolls the run up
   (`done | failed | cancelled`) only once every node has settled. Both
   forms are idempotent — a duplicate cancel of an already-terminal
   node/run reports it settled rather than erroring.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Common codes: `run_not_found`, `state_unreadable`, `schema_mismatch`.
Always read the `code`; the `message` is human prose and may change.

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
  installer). Stop and wait — `run list` / `run show` payload shape
  may have changed.
- **Newer than `{{CLI_VERSION}}`**: the installed binary is ahead of
  what this skill was written for. The whole bundled skill catalog has
  moved with the binary, so refresh all of them:
  `orchestratectl skill install --force` (add `--agent codex` for Codex
  or `--agent all` for both). To refresh only this skill, run
  `orchestratectl skill install octl-run-overview --force`. Continue
  once the skills match.
- **Equal**: proceed normally.
