---
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Stint shared reference — Execution DAG convention, operating policy, prerequisites

This is the shared reference for the stint skill family (`stint-start` runs it every
round; `stint-handoff` does the terminal wrap). It ships **alongside** `stint-start`'s
`SKILL.md` — both skills install it into `~/.claude/skills/stint-start/` — so an agent
running either skill in any project can open it. Both `stint-start` and `stint-handoff`
LINK here for the Execution-DAG convention, the operating-policy facts to read, and the
project prerequisites, so those rules live in exactly one place and cannot drift between
the two skills.

Everything here is **generic**: the notation and rules are defined once; the actual
lanes, hot files, deploy command, and other project facts are read from the repo's own
`AGENTS.md` / `TODO.md`, never hardcoded.

## Reading the operating policy

At the start of every round (`stint-start` Phase 0), read the repo's operating facts from
its root `AGENTS.md` and `CLAUDE.md`, and its `TODO.md` handoff block:

- deploy command + deploy autonomy, deploy target
- green-gate commands (typecheck / build / smoke)
- a way to check what's live (live-version check)
- the hot-file list (these define the DAG's lanes)
- migration rules
- test-account reset preference
- the `TODO.md` handoff block (`## 🔄 Continue here` / `ALOITA TÄSTÄ`) — where the last
  session left off and what prod is running

If a needed fact is missing, ask the user and offer to document it (see *Project
prerequisites* below). The skill hardcodes no project facts — it relies on the repo
documenting its own operating manual.

## Execution DAG (the convention)

The DAG is the round's **scheduling plan**, living in a delimited block in `TODO.md`. It
is generic: this reference defines the *notation and rules*; the actual lanes and hot files
are **project facts** in the repo's `AGENTS.md` / `TODO.md`, never hardcoded here.

**What it owns vs. what it reads.** The DAG is authoritative for the *plan* — each issue's
lane, the order within a lane, and cross-lane `collision:` tags. It stores **no status**;
`issuectl` is authoritative for status, read on demand. No fact lives in both, so they
cannot drift. You **merge** the DAG (at the start of every `stint-start` round and again
at the final `stint-handoff` wrap) — never regenerate it from scratch, which
would force re-deriving the hot-file collision matrix and risk hallucinating or dropping a
`collision:` edge.

**Lanes = hot-file families.** Two issues whose fixes touch the same hot file share a lane
and are sequenced (≤1 live worktree per lane at a time). Disjoint issues sit in different
lanes and run in parallel. `UNLANED` means **confirmed to touch no hot file** (parallel-safe)
— when in doubt, lane it, don't UNLANE it. Deferred / not-in-plan issues live under
`## Adjacent backlog` placed **outside** the `<!-- execution-dag:end -->` delimiter, **not**
a lane and **not** inside the fenced block (or the drift check flags them every merge).

**Canonical block** (delimited so tooling and the `comm -3` check parse only the nodes):

````markdown
## Execution DAG (<YYYY-MM-DD>)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge each round (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: <slug>   ← start here on resume
LANE A — <hot-file family>
  ▶ <slug-a1>
    <slug-a2>   after <slug-b2> (needs its new API)
LANE B — <hot-file family>
  ▶ <slug-b1>
    <slug-b5>   collision: <shared-hot-file>
UNLANED — confirmed no shared hot files, run anytime (one slug per line):
    <slug-x>
    <slug-y>
```
<!-- execution-dag:end -->
````

**Merging the DAG against reality** (Phase 0 of every round, and the final merge in
`stint-handoff`). This is a stateful **merge**, not a rewrite: preserve the existing lane
assignment, order, and `collision:` tags; only reconcile the *set* of issues.

- Fetch the **active set** = every issue whose `issuectl` status is **non-terminal**
  AND which is not held out of the plan by a lifecycle label (`deferred` or
  `needs-triage`). For the default schema the status part is `--status open` **unioned
  with** `--status in-progress` (`open` alone silently drops every live worktree's issue);
  add `--status testing` or any other non-terminal status the project's issue schema
  defines. Absence from the active set means "not active", not proof of "done" — only a
  terminal status drops a line.
  - **`needs-triage` (untriaged intake) is NOT in the active set** — an intake bug the bot
    filed is `open` and non-terminal, but it has not been admitted to the plan yet.
    Excluding it here is what makes the handoff's human-ack gate real: without it, this
    merge would silently sweep every unacked intake bug into a lane. `/stint-handoff`
    admits an acked item by removing `needs-triage` (so it *enters* the active set); until
    then it stays out of the DAG entirely (not even in `## Adjacent backlog`).
- **Drop** DAG node lines whose slug is not in the active set (landed / renamed / closed /
  still `needs-triage`).
- **Add** active issues missing from the DAG into their lane. Assign the lane by which
  hot-file family the fix touches; if you **can't tell**, sequence it conservatively in
  the most-likely lane — do **not** default to `UNLANED`, which asserts *touches no hot
  file* (parallel-safe). Park `deferred`-labelled issues under `## Adjacent backlog`
  **outside** the `<!-- execution-dag:end -->` delimiter, not in a lane. A fast scoped
  drift check (extracts only the leading node slug per line, ignores prose/tags, and
  drops deferred + still-untriaged intake from the active side):
  ```bash
  comm -3 \
    <( { issuectl --json ls --status open; issuectl --json ls --status in-progress; } \
         | jq -r '.[] | select(.type != "epic" and (((.labels // []) | index("deferred")) | not) and (((.labels // []) | index("needs-triage")) | not)) | .slug' \
         | sort -u ) \
    <( sed -n '/execution-dag:begin/,/execution-dag:end/p' TODO.md \
         | sed -nE 's/^[[:space:]]+(▶[[:space:]]*)?([a-z0-9][a-z0-9-]*)([[:space:]].*)?$/\2/p' \
         | sort -u )
  ```
  (Epics, `deferred`, and still-`needs-triage` intake issues aren't lane nodes, so they're
  excluded from the active side. Left-only = active issues missing from a lane; right-only
  = stale DAG lines.)
- **Validate:** every `after`/`blocked_by` target resolves to a real issue; no
  self-dep; no cycle. A dangling or cyclic edge **invalidates the DAG** — surface it and
  repair it before selecting or spawning any DAG-picked work; never render a wrong head
  silently.
- **Recompute the head-of-line** (below). The **calling skill owns the `git commit`** —
  this procedure produces the reconciled `TODO.md` (plus any issue files `issuectl`
  rewrote; `issuectl` does not auto-commit) and hands that changed-file list to the phase
  that invoked it (`stint-start` Phase 0, or `stint-handoff` step 2), which commits by
  exact path — never `git add -A`. Do **not** commit here, or you double-commit when the
  caller commits again.

**Head-of-line (compute on read, never trust the printed `▶`):**

- An issue is **eligible** iff it is in the active set (which already excludes `deferred`
  and still-`needs-triage` intake) and every `blocked_by` target is **delivered** —
  `status ∈ {fixed, done}`. Any **other** terminal status (`wontfix` / `obsolete` /
  `cannot-reproduce` / `duplicate`) does **not** satisfy the dependency (the code was never
  built) — the dependent stays blocked; flag it to the user.
- **`in-progress` does NOT exclude an issue — surface it, aggressively.** `in-progress`
  means STARTED, not "being worked right now": the status alone does not prove a live run
  currently owns the issue. A started-but-unfinished issue with no launched-but-unsettled
  run this round is exactly a **resumable candidate** to pick back up — never a reason to
  skip it. Double-work prevention is **not**
  the eligibility rule; it is the caller's **reserve-at-launch / claim** responsibility: a
  run launched **this round** holds its issue (and its collision files) until it settles,
  even before its first commit flips the issue to `in-progress` (see the spawnable rule
  below and `stint-start` Phase 2 — reserve at launch, not at first commit). That guard,
  not the head-of-line predicate, is what prevents two workers grabbing the same issue.
- `head(lane)` = the first eligible issue in that lane's order.
- A head is **spawnable** iff no launched-but-unsettled run **this round** already covers
  its issue AND none of its collision files (its lane's hot-file family + any `collision:`
  tag) is held by a live *or* launched-but-unsettled run (see `stint-start` Phase 2
  (Orchestrate) — reserve at launch, not at first commit). This is the sole double-work
  guard: an already-`in-progress` issue with no launched-but-unsettled run this round is a
  resumable head, so it *is* spawnable (you resume it).
- `GLOBAL HEAD-OF-LINE` = pick among spawnable heads: an explicit handoff "start here"
  first, else highest `issuectl` priority, else the top-most lane in the file (then its
  first eligible item), else slug order.

**Slugs, not positional codes.** Reference issues by slug everywhere (no `A1`/`B5`);
positional numbers churn on every insert. Lane letters are just coarse group labels.

Requires the repo to keep the delimited `## Execution DAG` section in `TODO.md` and a
hot-file list in `AGENTS.md` (see *Project prerequisites*).

## Project prerequisites (what the repo's AGENTS.md should provide)

The stint skills are generic, so they rely on the repo documenting its own operating facts.
If any are missing, ask the user and offer to add them:

- **Deploy command + target** — the exact one-liner, and whether it targets a
  test/staging server or production.
- **Deploy autonomy** — may the conductor deploy without asking, or is go/no-go
  required?
- **Live-version check** — how to see what's currently deployed (health endpoint,
  image tag, etc.), so Phase 0 can compare against main.
- **Green gate** — the typecheck/build/smoke commands that must pass before deploy.
- **Hot files** — shared files that must be sequenced, not parallelised. These define the
  DAG's lanes (a lane = a hot-file family), so the list needs to be specific enough to map
  an issue's likely-touched files to a lane.
- **A delimited `## Execution DAG` section in `TODO.md`** — the `<!-- execution-dag:begin
  -->` / `end` block the DAG convention maintains. If absent, create it on the first merge.
- **Test-account reset preference** — if testing should start from a known state.
