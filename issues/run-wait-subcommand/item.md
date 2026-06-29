---
created: 2026-06-29
updated: 2026-06-29
type: improvement
status: open
priority: high
---

# Add 'orchestratectl run wait' completion-blocking primitive

## Description

Add a `run wait <run-id>...` subcommand that **blocks until one or more
runs reach a terminal state** and emits a structured summary, replacing
the hand-rolled poll loops every SKILL and agent currently writes.

Proposed in /tmp/orchestratectl-run-wait-proposal.md (2026-06-29) as a
follow-up to two real failures on a deutschpad orchestration session:
multi-run zsh word-split bug (see issue
`skill-multi-run-poll-zsh-unsafe`) and the long-standing
"poll `manifest.status`, not `lifecycle`" footgun.

## Problem this solves

Every caller that wants to "do X after the spinoff finishes" hand-rolls
a poll loop around `orchestratectl run show ... | jq '.data.manifest.status'`.
This is fragile and has produced real failures:

1. **Shell-portability bugs** — `for id in $ids` silently breaks under
   zsh. Each skill/agent re-implements the loop and re-introduces the
   bug.
2. **Wrong-field bugs** — callers poll raw JSON and pick the wrong
   field (e.g. `lifecycle` instead of `manifest.status`). The repo
   `CLAUDE.md` "state integrity invariants" section exists in part
   because of this exact recurrence.
3. **Cadence guesswork** — every caller picks a `sleep N`. No shared
   backoff policy.
4. **No multi-run primitive** — fan-out/parallel drivers need "block
   until ALL of these N runs settle"; the absence of a primitive
   pushes everyone into the buggy `for` loop.

The supervisor already knows precisely when a run goes terminal. The
binary should expose a blocking wait so callers stop reconstructing
that knowledge from file polls.

## Synopsis

```
orchestratectl run wait <run-id>... [flags]
```

## Flags

- `--all` (default) — return when **every** listed run is terminal.
- `--any` — return as soon as **one** listed run is terminal.
- `--timeout <dur>` — give up after e.g. `30m`; exit code distinguishes
  timeout from terminal.
- `--output json|jsonl|text` — final summary (default `json`).
- `--progress` — emit one `jsonl` line per state transition to stderr
  for live UIs.
- `--fail-on-error` — exit non-zero if any run settled as
  `failed`/`cancelled` (default: exit 0 as long as all reached *some*
  terminal state).
- `--poll-interval <dur>` — override internal cadence (default: binary
  picks event-driven / sane backoff; callers shouldn't need this).

## Exit codes

- `0` — wait condition satisfied (`--all` → every run terminal;
  `--any` → ≥1 terminal). With `--fail-on-error`, all were `done`.
- `2` — timeout reached before wait condition met.
- `3` — `--fail-on-error` and wait condition met but ≥1 run was
  `failed`/`cancelled`.
- `1` — usage / unknown run id / internal error.

## Output (json)

```json
{
  "schema_version": 1,
  "data": {
    "waited_ms": 412345,
    "condition": "all",
    "runs": [
      {"run_id": "01k...", "status": "done",   "merged": true,  "summary": "..."},
      {"run_id": "01k...", "status": "done",   "merged": true,  "summary": "..."},
      {"run_id": "01k...", "status": "failed", "merged": false, "error": "merge_failed: ..."}
    ]
  }
}
```

Folds in the terminal `node report` summary so the caller gets the
outcome without a follow-up `node show`.

## Smallest viable first cut (v0.1.0 scope)

If a full event-driven implementation is too much for the v0.1.0
release window, implement `run wait` as an **internal** poll of
`manifest.status` with sane backoff plus `--all` / `--any` /
`--timeout` and the JSON summary. Even this thin version removes the
entire bug class — the (correct, tested) loop lives in the binary
instead of in every caller's shell.

Acceptance criteria for v0.1.0:

- subcommand exists, --json envelope honoured, exit codes per above
- integration test in `crates/octl-cli/tests/` covering `--all`
  success, `--any` race, `--timeout` exit code, `--fail-on-error` exit
  code
- every `worktree-*` SKILL.template.md's "Following progress" section
  replaced with a single `orchestratectl run wait` call
- `orchestratectl skill install --force` redeployed
- closes `skill-multi-run-poll-zsh-unsafe` as superseded

## Migration / docs

- Add `run wait` to the `worktree-*` skills' "Following progress"
  sections and **replace** the hand-rolled `while ... run show ...
  case` snippet.
- Keep `run show` for one-shot inspection; `run wait` is the blocking
  counterpart.

## Interaction with agent harnesses

A blocking `run wait` plays well with background execution: the agent
runs `orchestratectl run wait <ids...> --all` as a single background
command, the harness notifies on process exit. One process, one exit,
one notification — strictly better than a hand-rolled background poll.

## Source

Proposal at `/tmp/orchestratectl-run-wait-proposal.md` (transient — copy
into this issue's `plan.md` if a longer-form design pass is needed
before implementation).

