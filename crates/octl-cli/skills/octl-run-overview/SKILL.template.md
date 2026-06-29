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
- `orchestratectl run show <run-id>` — one run, full detail

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
        "kind": "worktree-code | spinoff | fan-out | orchestrate",
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

- `kind` — picks the right follow-up command (a `fan-out` resumes
  differently from a `worktree-code`).
- `lifecycle` — the run's **category**, derived from `kind`. Always one
  of `autonomous` (fire-and-forget — spinoff, research, bugfix, …) or
  `interactive` (`code`, `orchestrate`). It is NOT a progress state and
  never transitions to a terminal value.
- `status` — the **terminal-progress field**. Values are `pending`,
  `running`, `done`, `failed`, `cancelled`. **Terminal states are
  `done | failed | cancelled`** — once any of those is set the run is
  settled (the reducer freezes further status changes). Branch on this
  to detect completion.

## `run show` payload

`data.manifest` extends the summary with detail; `data.counts` carries
denormalised counters; some kinds add kind-specific fields.

```json
{
  "data": {
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
