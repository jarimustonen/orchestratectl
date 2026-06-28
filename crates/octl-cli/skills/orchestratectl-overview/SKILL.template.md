---
name: orchestratectl-overview
description: First read for any agent that has just discovered the `orchestratectl` binary mid-conversation. Teaches the overall shape of the tool — runs, supervisors, nodes, discussions, spinoffs — and the canonical create→supervise→collect-reports cycle. Use when asked "what is orchestratectl", when the binary appears in a session for the first time, or before issuing the first non-trivial command.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# orchestratectl-overview

`orchestratectl` (binary name: `orchestratectl`) is the state owner for
AI-agent workflows: worktrees, fan-outs, orchestrations, and spinoffs.
Every workflow is a **run** with canonical state under
`~/.orchestratectl/runs/<run-id>/`. Read this skill once at the start of
any session that touches the tool — every other `octl-*` skill assumes
the vocabulary and conventions defined here.

## Output contract (read this first)

Every machine-readable command emits the canonical envelope:

```json
{"schema_version": 1, "data": {...}, "warnings": ["..."]}
```

- The default `--output` is `jsonl` — one compact envelope per line on
  stdout. AI agents parse it directly with `serde_json::from_str` per
  line.
- `--output json` returns a single pretty-printed document; use it for
  one-shot inspection.
- `--output text` is the human summary; do not parse it.
- Errors print a separate envelope on **stderr** with non-zero exit:
  `{"schema_version": 1, "error": {"code": "<snake_case>", "message": "..."}}`.
  Always branch on `error.code`; the message is human prose.

If `schema_version` is a value you do not recognise, refuse to proceed
— the data shape may have changed under you.

## The verbs you will use most

- `orchestratectl version` — binary version, commit, schema versions,
  and the bundled skill catalog (see §17 of the design doc). Run this
  first when you discover the binary; it tells you whether the skill
  you loaded matches the binary on disk.
- `orchestratectl skill list` / `skill print <name>` / `skill install <name>`
  — discover, stream, and persist the operating manual for each
  workflow.
- `orchestratectl run create --kind <kind> --title "..." --task "..."`
  — start a new run (kinds: `worktree-code`, `spinoff`, `fan-out`,
  `orchestrate`). The brief is `--task "<inline>"` or, for a long brief,
  `--prompt-file <path>` — there is no `--prompt` flag. See the
  `octl-spawn-spinoff` skill for the spinoff specifics.
- `orchestratectl run list` / `run show <id>` — inspect runs. See the
  `octl-run-overview` skill for payload shapes.
- `orchestratectl supervise <run-id>` — the long-lived per-run
  supervisor process; `run reattach` invokes it. Most agents do not call
  this directly.
- `orchestratectl node list` / `node show <id>` / `node report` —
  per-unit detail inside a run, and the structured terminal report a
  spinoff submits when it merges itself back. Node ids have the form
  `n-` followed by 4–10 ASCII digits (e.g. `n-0001`, the first node of
  every run); the binary rejects any other shape. Never invent a slug
  like `n-driver-001` — discover the id from `run create`'s `node_id`
  field or from `node list`.
- `orchestratectl discussion list` / `discussion resolve` —
  human-blocking decisions a worker raised; agents resolve these
  before the run can continue.
- `orchestratectl spinoff list` / `spinoff approve` / `spinoff reject`
  — spin-off proposals from worker runs that need a human sign-off
  before becoming real runs.

## Canonical cycle: create → supervise → collect reports

The flow every workflow follows:

1. **Create**: `orchestratectl run create --kind <kind> --title
   "<title>" --task "<the brief>"` returns a `data` payload with
   `run_id`, `kind`, and `lifecycle` (the kind's classification —
   `autonomous` or `interactive`, not a progress value). The run's
   progress starts at `status: pending`, read later via `run show`.
2. **Supervise**: a background supervisor process picks the run up and
   drives the worker agent(s). State transitions land in the run's
   event log; `run show <id>` reads them.
3. **Collect**: when the worker finishes, it submits a structured
   `node report` describing the outcome (success, failures, spin-off
   proposals, discussion items, wrap-up recommendations). The
   orchestrator reads these via `node show` and `discussion list`.

Two distinct fields, two distinct meanings — do not conflate them:

- **`lifecycle`** carries the run's *classification*, derived from its
  kind and fixed for the run's whole life: `autonomous` (spinoff,
  fan-out, research, bugfix, technical-decision, make-skill,
  orchestrated — runs to completion unattended) or `interactive`
  (`code`, `orchestrate` — human-driven in a tmux window). It is **not**
  a progress signal: a brand-new orchestrated child already reads
  `lifecycle: autonomous`.
- **`status`** carries the run's *progress* and is the field to read
  when deciding whether a run is finished: `pending` / `running` /
  `blocked` / `done` / `failed` / `cancelled`. `done`, `failed`, and
  `cancelled` are terminal. Read `status` (via `run show <id>` →
  `data.manifest.status`), never `lifecycle`, to tell whether work is
  complete.

## When to use which skill

- **`orchestratectl-overview` (this one)** — sanity-check vocabulary,
  confirm the canonical cycle, locate the right specific skill.
- **`octl-run-overview`** — when you need to read `run list` / `run show`
  output and reason about a run's state.
- **`octl-spawn-spinoff`** — when the user wants to spawn one focused
  autonomous task without interactive review.

If the user asks for behavior these skills do not cover (interactive
worktree review, fan-out, orchestrate), call `--help` first and report
back rather than guessing.

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
  installer). Stop and wait — schema / CLI surface may have changed.
- **Newer than `{{CLI_VERSION}}`**: the installed binary is ahead of
  what this skill was written for. The whole bundled skill catalog has
  moved with the binary, so refresh all of them — not just this one:
  `orchestratectl skill install --force` (default agent is `claude`;
  pass `--agent codex` for Codex or `--agent all` for both). To refresh
  only this skill, run
  `orchestratectl skill install orchestratectl-overview --force`.
  Continue once the skills match.
- **Equal**: proceed normally.
