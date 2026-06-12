---
name: octl-run-overview
description: Read the output of `orchestratectl run list` and `orchestratectl run show` to inspect the state of orchestrated agent workflows (worktrees, fan-outs, spinoffs). Use when asked about run status, when triaging an in-flight orchestration, or before deciding whether to spawn, resume, or abort work.
version: 1
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

- `orchestratectl run list [--json]` — every run, newest first
- `orchestratectl run show <run-id> [--json]` — one run, full detail

Always pass `--json`. The human format is for terminals; agents read
JSON.

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
        "lifecycle": "pending | running | paused | completed | failed | cancelled",
        "status": "<kind-specific short label>",
        "created_at": "2026-06-12T10:30:00Z",
        "updated_at": "2026-06-12T10:45:12Z"
      }
    ]
  }
}
```

Fields that drive decisions:

- `kind` — picks the right follow-up command (a `fan-out` resumes
  differently from a `worktree-code`)
- `lifecycle` — the only field that tells you whether the run is
  finished. **Never assume `completed` from `status` alone**; only
  `lifecycle` is authoritative.
- `status` — a short human label (e.g. `"3/10 units done"` for fan-out).
  Useful in summaries; do not branch on its text.

## `run show` payload

`data.run` extends the summary with detail: structured per-unit
progress, the originating prompt, the merge target, and event log
pointers. Shape depends on `kind`; always check `kind` before reading
kind-specific fields.

```json
{
  "data": {
    "run": {
      "id": "01HZ...",
      "kind": "fan-out",
      "lifecycle": "running",
      "status": "3/10 units done",
      "created_at": "...",
      "updated_at": "...",
      "units": [
        { "id": "u-001", "lifecycle": "completed", "branch": "fan-out/u-001" }
      ]
    }
  }
}
```

## Decision rules

1. **Triaging "is this still going?"** — read `lifecycle`. Anything other
   than `running` or `pending` is not actively progressing.
2. **Deciding whether to spawn more work** — list runs first. If a
   `fan-out` is already `running` on the same scope, do not start a
   second; resume or wait.
3. **Surfacing problems to the user** — when `lifecycle == "failed"`, the
   event log has the cause; quote it instead of guessing.
4. **Schema drift** — if `schema_version` does not match what this skill
   describes, stop and tell the user the skill is out of date with the
   installed binary.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Common codes: `run_not_found`, `state_unreadable`, `schema_mismatch`.
Always read the `code`; the `message` is human prose and may change.
