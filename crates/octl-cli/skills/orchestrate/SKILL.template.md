---
name: orchestrate
description: Plan and execute a set of heterogeneous, dependency-ordered features with meaningful parallelism. Spawns one top-level driver run via `orchestratectl run create --kind orchestrate` and then one autonomous child per ready feature via `--kind orchestrated --parent-run-id <id> --parent-node-id <id>`. Runs autonomously by default — bold first, ask later — logging every judgment call to the parent run's event log and writing a hierarchical `report.yaml` + `report.md` to `~/.orchestratectl/runs/<id>/` for the user to read at the end. Pauses (audible alert) ONLY when a decision would massively constrain remaining children, when something genuinely cannot be done, or when the orchestrator is outside its own competence. Use when the user asks to "drive this whole thing", "deliver feature X end-to-end", or "ship this campaign". NOT for software/container orchestration (Docker/Kubernetes), N identical units (`/fan-out`), one interactive task (`/worktree-code`), one autonomous task (`/worktree-spinoff`), or a fully serial chain (`/worktree-code` in sequence).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# orchestrate

An **orchestrated campaign** is a heterogeneous set of features the
user wants delivered together, with dependencies between them and
meaningful parallelism where the dependencies allow it. The
orchestrator (this skill, running in the user's main conversation)
plans the DAG, spawns one autonomous worker per ready feature via
`--kind orchestrated`, watches the parent run's event log, logs every
judgment call it makes along the way, and finally writes a
**hierarchical report** the user reads in product terms — not as ten
raw worker dumps.

The user has explicitly asked for a **bold-by-default** orchestrator:
keep moving, record the decisions, present them at the end. Pakkopysäytys
(forced stop with audible alert) is reserved for three specific
triggers documented in §"When to pakkopysäytys" below.

Read `orchestratectl-overview` first; read `worktree-orchestrated` to
understand the child-spawn contract; read `fan-out` for the contrast —
fan-out is N identical units, orchestrate is a DAG of distinct
features.

## When to use

- ✅ "Drive this whole feature campaign", "deliver this end-to-end",
  "ship this initiative" — anything that decomposes into 3+ distinct
  features with some interdependence.
- ✅ Issue tagged `epic` plus an explicit ask to execute its
  child-issue breakdown.
- ❌ Software/container orchestration (Docker, Kubernetes). This
  skill is about agent-work coordination only.
- ❌ N identical independent units → `/fan-out`.
- ❌ One feature, interactive review → `/worktree-code`.
- ❌ One feature, fire-and-forget → `/worktree-spinoff`.
- ❌ Fully serial chain with zero parallelism → just sequence
  `/worktree-code` calls; orchestrate's planning overhead is wasted.
- ❌ Pure research or pure decision-record → use the matching
  `/worktree-research` / `/worktree-technical-decision`.

## The boldness contract (read this before designing the workflow)

The orchestrator MUST internalize this before it spawns anything:

- **Default behavior is to continue.** When a worker surfaces a
  judgment call, a discuss item, a small ambiguity, an unforeseen
  cleanup, an alternative path — the orchestrator picks the most
  defensible option using the campaign brief as its north star,
  records the choice (see §"Decision logging"), and keeps going. It
  does NOT page the user.
- **The user is the reviewer, not the driver.** The orchestrator
  presents the full set of decisions at the end. The user audits
  them, redirects if needed, and the cost of a wrong-but-recorded
  decision is "we discuss it after, maybe re-run a worker" — not
  "the campaign sat idle for hours".
- **Pakkopysäytys is rare and expensive.** Each pause has an audible
  alert and blocks the campaign until the user responds. Use the
  three triggers in §"When to pakkopysäytys" — nothing else.
- **Every autonomous choice is logged.** The contract with the user
  is "you can read everything I decided". A choice that does not
  appear in `report.yaml` is a contract violation, not a convenience.

## When to pakkopysäytys (the only three triggers)

Pakkopysäytys = emit `discuss.critical` event on the parent run, play
an audible alert (macOS `afplay /System/Library/Sounds/Sosumi.aiff`
or equivalent), block the campaign until the user resolves the
discussion via `orchestratectl discussion resolve <id>`.

**Trigger 1 — cross-cutting decision with days-of-rework cost.**
A choice whose answer would dictate the shape of multiple downstream
workers, where "we picked wrong" means re-running several of them.
Examples:
- Picking a public-API contract that 4 sibling workers will then
  implement against.
- Choosing a data-model shape that downstream features depend on.
- Selecting an external service / library that future workers will
  integrate.

Heuristic: if the orchestrator's mental rollback cost for the wrong
choice is "the rest of this campaign needs to start over", pakkopysäytys.
If it is "we re-run this one worker", keep going and log it.

**Trigger 2 — something genuinely cannot be done.**
A required input is missing (file does not exist, dependency does not
compile, the upstream worker produced an unusable artefact), and no
amount of orchestrator judgment can route around it. The user must
intervene with new direction — pick a different path, supply the
missing piece, or call off that branch.

Distinct from a worker failing: a worker that produces a
`node report.success: false` with a clear cause is a recorded outcome,
not a pakkopysäytys. The orchestrator decides whether to retry, skip,
or escalate. Pakkopysäytys triggers only when the orchestrator itself
has no good option.

**Trigger 3 — orchestrator outside its competence.**
The orchestrator recognizes the choice is genuinely beyond what a
text-pattern-matching agent should decide unilaterally: regulatory
implications, irreversible production changes, deletion of
non-recreatable artefacts, financial-cost decisions. Surface to the
user; do not guess.

Anything that does not fit these three: log the decision, continue.

## Workflow

### 0. Validate context

1. Working directory must be a git repo with a clean current branch.
   Per repo CLAUDE.md, uncommitted state on the source branch must be
   resolved before spawning.
2. `orchestratectl version --output json` once per session; confirm
   `.data.version` matches `{{CLI_VERSION}}`.
3. Capture the current branch — it becomes the **source branch** the
   integration branch forks from.
4. Confirm the user is asking for orchestration, not for one of the
   simpler patterns. If in doubt, summarize the plan you would build
   and ask **once** for a go/no-go on the shape. This is NOT
   pakkopysäytys — it is upfront scoping.

### 1. Plan the DAG

This is the orchestrator's first substantive job. Build a plan with:

- **Campaign goal** — one paragraph; the product-level outcome.
- **Features** — each is one autonomous worker run. For each:
  - id (short slug)
  - title (one line, product-language)
  - brief (self-contained — the worker cannot ask follow-ups; see
    `worktree-orchestrated` for what a brief contains)
  - depends_on: [] (ids of features that must complete first)
- **Integration branch name** — the shared branch every worker merges
  into. Default: `orchestrate/<campaign-slug>-<ISO-date>`.
- **Parallelism estimate** — how many workers can run simultaneously
  once dependencies allow. Default cap: 5.

Plan output goes into the campaign brief that creates the driver
run (next step) AND becomes the initial scaffolding of
`report.yaml`'s `features:` section.

If the DAG has unresolved ambiguity that would force pakkopysäytys
within the first worker, ask the user **once** at this step rather
than spawning and immediately pausing. Otherwise, proceed.

### 2. Create the driver run

```
orchestratectl run create \
  --kind orchestrate \
  --title "<campaign-slug>" \
  --task "<campaign goal + full DAG, JSON or YAML inline>" \
  --source-branch <current-branch> \
  [--idempotency-key <key>]
```

> **About the driver run.** `--kind orchestrate` is the top-level
> driver kind. It does NOT spawn a worktree (`lifecycle: interactive`,
> the orchestrator agent runs in the user's main conversation), only a
> run dir under `~/.orchestratectl/runs/<id>/` to hold the event log,
> manifest, and final report files. The success envelope's
> `supervisor` field reads `"orchestrator-in-main-conversation"` —
> that is correct; you are the supervisor.

Capture `data.run_id` — this is the **driver run id**. Every child
worker references it via `--parent-run-id`. Also capture
`data.node_id` — every child references it via `--parent-node-id`. For
a fresh driver run it is always `n-0001` (the driver node), but read it
from the envelope rather than hard-coding it.

### 3. Create the integration branch

```
git branch <integration-branch> <source-branch>
```

Workers merge into this branch (NOT directly into the source branch)
so the user can review the whole campaign as one diff at the end.
Record the branch in `report.yaml`'s top-level metadata.

### 4. Fan out ready features

While there are features not yet `done` or `failed`:

1. Find every feature whose `depends_on` set is fully `done` and that
   is not currently `running`.
2. Up to the parallelism cap, spawn one child per ready feature:

   ```
   orchestratectl run create \
     --kind orchestrated \
     --title "<feature-id>" \
     --task "<self-contained feature brief>" \
     --source-branch <integration-branch> \
     --parent-run-id <driver-run-id> \
     --parent-node-id <driver-node-id> \
     --idempotency-key <campaign-slug>-<feature-id>-v1
   ```

3. Tail the parent's event log:

   ```
   orchestratectl event tail <driver-run-id> --follow
   ```

   Events to act on:
   - `child.spawned` — confirm the new child appears in the manifest.
   - `child.lifecycle running|completed|failed` — update the DAG.
   - `child.report` — the worker's structured terminal report.
     Read it, decide next action (next ready features, retry, skip).
   - `discuss.critical` (from a worker) — this IS a pakkopysäytys for
     the campaign; jump to §"Handling pakkopysäytys".

4. On a child `completed` with `node report.success: true`: mark its
   feature `done`, append its report to `report.yaml`, loop back to
   step 1.

5. On a child `completed` with `node report.success: false`: the
   orchestrator decides per the boldness contract. Options:
   - **Retry once** with the same `--idempotency-key + "-r2"` if the
     failure looks transient. Log the decision.
   - **Skip and continue** if downstream features do not strictly
     need this one. Log the decision plus the consequence (which
     downstream features lose access to what artefact).
   - **Pakkopysäytys** only if Trigger 1, 2, or 3 applies.

6. On a child `failed` (supervisor crash, not a structured report):
   treat as transient; retry once. Persistent failures are Trigger 2
   pakkopysäytys.

### 5. Decision logging

Every autonomous choice the orchestrator makes — large or small — gets
appended as an event on the driver run's log via `event create`:

1. Write the decision payload to a temp file:

   ```bash
   cat > /tmp/decision-<id>.json <<'JSON'
   {
     "id": "d-NNN",
     "summary": "<one line>",
     "options": ["A", "B"],
     "chose": "A",
     "because": "<reason>",
     "scope": "<which feature(s) this affects>",
     "reversibility": "low|medium|high"
   }
   JSON
   ```

2. Append it to the driver run's event log:

   ```
   orchestratectl event create <driver-run-id> \
     --kind orchestrator.decision \
     --from-file /tmp/decision-<id>.json \
     --idempotency-key d-NNN
   ```

3. Remove the temp file.

Decision IDs are stable strings (`d-001`, `d-002`, ...) so
`report.yaml` can reference them and the future UI can link to them.
The `--idempotency-key` (matching the decision id) makes the append
safe to retry if the orchestrator restarts mid-write.

### 6. Handling pakkopysäytys

When any of the three triggers fires:

1. Append a `discuss.critical` event to the driver run's log via
   `event create`. The payload includes:
   - `summary` — one sentence describing what is blocked.
   - `trigger` — `cross_cutting | cannot_do | out_of_competence`.
   - `options` — what the orchestrator considered; never present
     fewer than two unless the situation is binary.
   - `recommended` — the orchestrator's lean (still useful even if
     the user overrides).
   - `affected_features` — which downstream features will sit idle
     until resolved.

   ```bash
   cat > /tmp/discuss-<id>.json <<'JSON'
   { ... payload ... }
   JSON
   orchestratectl event create <driver-run-id> \
     --kind discuss.critical \
     --from-file /tmp/discuss-<id>.json \
     --idempotency-key <discuss-id>
   ```
2. Play the audible alert (macOS: `afplay
   /System/Library/Sounds/Sosumi.aiff`; Linux: `paplay`/`aplay` on a
   short bell if available; silent fallback only if no audio path
   exists).
3. Surface the discussion to the user via:

   ```
   orchestratectl discussion list <driver-run-id>
   ```

   Tell the user what came up, your recommendation, and what is
   blocked.
4. Wait. The campaign does NOT progress until the user runs
   `orchestratectl discussion resolve <discussion-id> --choice <...>`
   (or instructs the orchestrator to choose for them — at which point
   the orchestrator chooses, records, and continues).
5. On resume, the orchestrator reads the resolution, applies it,
   resumes the loop from §4.

### 7. Final synthesis

When every feature is `done` or `failed`:

1. Generate `report.yaml` (machine-readable). Suggested top-level
   shape:

   ```yaml
   schema_version: 1
   campaign:
     slug: <campaign-slug>
     goal: <one paragraph>
     started: <ISO timestamp>
     finished: <ISO timestamp>
     source_branch: <branch>
     integration_branch: <branch>
   features:
     - id: f-001
       title: <line>
       status: done|failed|skipped
       worker_run_id: 01HZ...
       depends_on: []
       summary: <one paragraph synthesizing the worker's report>
       report_ref: <relative path to worker's node report>
   decisions:
     - id: d-001
       summary: <line>
       chose: <option>
       because: <reason>
       scope: [f-002, f-003]
       reversibility: low|medium|high
   pakkopysäytykset:
     - id: p-001
       trigger: cross_cutting|cannot_do|out_of_competence
       resolved_by: user|orchestrator-after-permission
       chose: <option>
       at: <ISO timestamp>
   discuss_for_user:
     - id: u-001
       summary: <line>
       why_now: <reason this surfaces to the user even though we did not pakkopysäytys>
   spinoff_candidates:
     - id: s-001
       suggested_kind: spinoff|code|research|...
       title: <line>
       rationale: <reason>
   ```

2. Generate `report.md` (human-readable). Mirror the YAML structure
   but as narrative prose, with the **top-level summary first** (the
   user reads this in the chat reply, drills in only if curious).
   Suggested sections:
   - `# <campaign title>` plus the goal paragraph.
   - `## What changed` — three to five bullets describing the
     campaign's effect on the product, not on the codebase.
   - `## Decisions worth knowing` — every decision with
     `reversibility != high`, plain prose, one per paragraph.
   - `## Things that came up` — `discuss_for_user` items.
   - `## Suggested follow-ups` — `spinoff_candidates`.
   - `## Per-feature detail` — a section per feature, drill-down.

3. Write both files to `~/.orchestratectl/runs/<driver-run-id>/`:
   - `report.yaml`
   - `report.md`

4. Reply to the user in the chat with **only** the `report.md`'s top
   two sections (`# <campaign>` and `## What changed`), plus a one-
   line pointer: "Full detail at `~/.orchestratectl/runs/<id>/report.md`
   (or read `report.yaml` machine-side)."

5. The user reviews the integration branch (`<integration-branch>`)
   themselves. The orchestrator does NOT auto-merge into the source
   branch — that is the user's final call via `/worktree-merge` or a
   direct `git merge`.

## Resume

If the orchestrator is interrupted mid-campaign (terminal closed,
session crashed):

1. The user (or a new orchestrator instance) runs
   `orchestratectl run reattach <driver-run-id>`.
2. The orchestrator rebuilds DAG state from the manifest + event log:
   features `done`, `running`, `failed`, `pending`, and the open
   discussions awaiting resolution.
3. If there is an open `discuss.critical` discussion, present it to
   the user first.
4. Otherwise, resume the fan-out loop from §4.

## Issue Management

The orchestrator owns issue interaction for the whole campaign so the
N workers do not race to update or close the same epic:

- On campaign start, if the user references an epic
  (`/orchestrate <epic-slug>`), read it via `issuectl --json show
  <slug>` and use it as the campaign goal source.
- During the campaign, append commits to the epic via
  `issuectl --json update <slug> --add-commit "<sha>:<feature-id>: <line>"`
  as each feature merges into the integration branch.
- On final synthesis, do NOT auto-close the epic. The user merges the
  integration branch first; closure follows merge, not orchestrate
  completion.

## Errors

Failures print a JSON envelope to **stderr** with non-zero exit:

```json
{"schema_version": 1, "error": {"code": "<code>", "message": "..."}}
```

Likely codes (driver-level — child-level codes belong to
`worktree-orchestrated`):

- `invalid_arguments` — missing/empty `--title` or `--task`, bad
  source branch.
- `branch_not_found` — `--source-branch` does not exist.
- `integration_branch_exists` — a prior campaign with the same slug
  left its integration branch around. Pick a new slug or
  `git branch -D <branch>` deliberately first.
- `idempotent_replay` — informational; key matched a prior driver
  run; resume rather than spawn.
- `supervisor_spawn_failed` — the parent run's supervisor could not
  start. Inspect `<dir>/supervisor.stderr.log` and consider `run
  reattach`.

Child errors come back as `child.lifecycle failed` events on the
parent log — the orchestrator handles them per §4 step 5–6, not as
top-level errors.

## Following progress

The orchestrator IS the follower for the children, but the user can
peek at any moment:

- `orchestratectl run show <driver-run-id>` — aggregate counts of
  features per state, open discussions, decision count.
- `orchestratectl event tail <driver-run-id> --follow` —
  authoritative live stream of children, decisions, and pakkopysäytys
  events.
- `orchestratectl discussion list <driver-run-id>` — open
  pakkopysäytys discussions (should be 0 most of the time).
- `cat ~/.orchestratectl/runs/<driver-run-id>/report.yaml` — current
  state of the in-progress report (regenerated on every milestone).

The future UI (planned, not yet built) will read `report.yaml` and
the event log to present a live drill-down of decisions and progress
without the user having to attach to terminals. The skill is designed
so that UI can land later without any change to the data model.

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
  `{{CLI_VERSION}}` and suggest upgrading. Stop — the
  `--kind orchestrate` driver kind, `event create`, `discussion
  resolve`, and `run reattach` semantics referenced by this skill may
  have changed.
- **Newer than `{{CLI_VERSION}}`**: refresh the catalog:
  `orchestratectl skill install --force` (or just `orchestrate
  --force`).
- **Equal**: proceed normally.

## Example

```
# User invokes from main conversation
/orchestrate Ship two-factor login for staff accounts — schema migration, backend endpoint, UI flow, email copy, ops runbook

# Orchestrator:
# - reads the brief, builds a 5-feature DAG (schema → endpoint → ui, parallel email + runbook),
# - creates the driver run and integration branch,
# - spawns the schema worker first; on its successful merge spawns endpoint;
#   spawns email + runbook in parallel from the start;
# - logs three small decisions along the way (rate-limit default, email subject phrasing, runbook split),
# - completes without pakkopysäytys,
# - writes report.yaml + report.md, replies in chat with the top section,
# - leaves the integration branch ready for /worktree-merge.
```
