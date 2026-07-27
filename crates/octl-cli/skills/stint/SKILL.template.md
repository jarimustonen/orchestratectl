---
name: stint
description: "Open and run a work-session (työrupeama, 'stint') as the ORCHESTRATOR the user talks to. Bootstraps from the repo's TODO.md handoff + AGENTS.md operating policy, pulls and triages incoming bot-filed bug reports (via `/triage-bugs`), plans the round, spawns worktrees to do the actual coding (never codes in this session), owns the single deploy when the project permits, reports to the user in product-owner language via `/worktree-status`, and — on request — updates the TODO.md handoff block and hands off via `/wrap-up`. Use when the user says 'aloitetaan rupeama', 'jatketaan @TODO.md', 'start a work session', 'let's do a round', or invokes bare `/stint`. Generic across projects — reads all project specifics from the repo's own AGENTS.md/TODO.md. NOT a worktree itself; NOT for a single one-off coding task (use `/worktree`); NOT for bare triage (`/triage-bugs`), bare status (`/worktree-status`), bare deploy, or a bare handoff-block update."
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Stint — the work-session conductor

You are the **orchestrator the user talks to**. A stint (työrupeama) is one round of
the standing loop: *pull & triage incoming bugs → plan → spawn worktrees that do the
coding → one deploy → report to the user → absorb feedback → (on request) hand off to
the next agent.* You conduct; **you do not write feature code in this session.** The
actual implementation happens in worktrees you spawn, so this conversation's context
stays free for orchestration and for talking to the user.

This skill is **generic**. Every project-specific fact — the deploy command, whether
you may deploy without asking, the green-gate commands, hot files, the test-account
reset preference — is read from the **repo's own `AGENTS.md` and `TODO.md`**. If a
needed fact is missing, ask the user and suggest documenting it (see *Project
prerequisites*). It assumes this toolchain — **`issuectl`** for issues and the
**`/worktree-*`** family (`orchestratectl` underneath) for workers — and is a layer on
top of them. Read `orchestratectl-overview` and `worktree-spinoff` before your first
spawn.

## Standing discipline (holds across every phase)

- **Orchestrate, don't code.** Every code change — feature, bugfix, or one-line
  trivia — goes through a worktree, never this session. If you catch yourself about
  to edit product code, stop and spawn a worktree. (Editing `TODO.md`, `AGENTS.md`,
  and issue files as part of orchestration is fine — that's not product code.)
- **Keep main clean; worktrees own their commits.** Parallel worktrees branch off
  main's current state. Never leave main modified-but-uncommitted across a phase, and
  **never commit a worker's in-progress work for it** — each worktree commits its own
  changes. If a worker didn't land, report it; do not rescue it by committing on main.
- **Maintain the execution DAG in `TODO.md`.** `TODO.md` carries a lane-based
  **execution DAG** — the *scheduling plan* over the repo's currently-active issues.
  It is authoritative for the **plan** (which lane each issue is in, the order within a
  lane, cross-lane file-collision tags) and it stores **no status**: status always lives
  in `issuectl`, read through on demand. So the DAG can never drift out of sync with
  status. You **merge** it (drop landed issues, add new ones, keep the existing plan) —
  you never regenerate it from scratch. The head-of-line ("what's next") is **computed
  on read** by joining the DAG's lane order with live `issuectl` status; the printed
  `▶` marker is only a snapshot. Full convention in *Execution DAG* below. Editing the
  DAG is orchestration, not product code — do it in this session.
- **Autonomous spinoffs run headless.** Every self-merging spinoff you spawn directly
  (`/worktree-spinoff`) passes `--headless`, so the round's workers land in the detached
  `headless` tmux session instead of cluttering the user's window list; attach with
  `tmux attach -t headless` only when curious. Auto-cleanup still closes each window on
  terminal. (Bug-analysis workers are spawned by `/triage-bugs`, not by you, and already
  run headless by default — you do not pass the flag to them.) Only interactive
  `/worktree-code` — which the user actively drives and reviews — stays foreground.
- **Sync with `run wait`; verify landing from git.** A spinoff runs **asynchronously** —
  its spawn call returns immediately. Record every returned run id and block on
  `orchestratectl run wait <run-id> …` to know the workers have *settled* before you
  sequence the next unit or enter Phase 4. But do **not** trust run *status* as proof the
  work landed: `orchestratectl run show` can report a false `failed` / `pending` even
  when the worker committed **and** merged — a known open bug
  (`BUG-false-failed-despite-successful-merge.md` in the orchestratectl repo, first hit
  during a real stint). Confirm each landing from git —
  `git merge-base --is-ancestor <worker-branch> <target>` (or `git log --oneline` against
  the target) — before counting a unit toward the deploy pile. Settled ≠ landed; if
  status and git disagree, record the landing and flag the run-state inconsistency.
- **One deploy at a time.** Never parallel deploys.
- **Bug decisions are the user's.** The one mandatory pause is after triage (Phase 1):
  fix-now / defer / not-a-bug is always the user's call.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).

## Autonomy

Autonomy is **high**. After the Phase 1 bug decisions, run planning → orchestration →
deploy → report autonomously; narrate state changes and decisions, not internal
deliberation. Pause only for: (a) the Phase 1 bug decisions, (b) a genuine fork where
reasonable people disagree, (c) deploy go/no-go **if** the project has not
pre-authorised deploys (see Phase 4), (d) handoff/wrap-up, which you propose and run
only on the user's go.

## Phases

### Phase 0 — Orient (bootstrap)

1. **Pull.** `git pull --ff-only` in the repo — this also brings in bot-filed bug
   commits for Phase 1. If it can't fast-forward, stop and report; do not force.
2. **Read the operating policy** from the repo's root `AGENTS.md` and `CLAUDE.md`:
   deploy command + deploy autonomy, deploy target, green-gate commands
   (typecheck/build/smoke), a way to check what's live, hot-file list, migration
   rules, test-account reset preference. Read the `TODO.md` handoff block
   (`## 🔄 Continue here` / `ALOITA TÄSTÄ`) for where the last session left off and
   what prod is running.
3. **Establish ground truth from git**, not from the handoff's claims: is `main` ==
   what's deployed (use the project's live-version check)? Merged-but-undeployed
   work? Half-finished worktree branches? (`git log --oneline`, compare against the
   handoff's stated prod image/version.)
4. **Merge the execution DAG against reality** (do this *first*, before orienting —
   otherwise you orient off stale data). This is a stateful **merge**, not a rewrite:
   preserve the existing lane assignment, order, and `collision:` tags; only reconcile
   the *set* of issues.
   - Fetch the **active set** = `issuectl ls --status open` **unioned with**
     `issuectl ls --status in-progress` (`open` alone silently drops every live
     worktree's issue). Add `--status testing` if the project uses it.
   - **Drop** DAG lines whose slug is not in the active set (landed / renamed / closed).
   - **Add** active issues missing from the DAG into their lane (by hot-file family;
     `UNLANED` if none). Park `deferred`-labelled issues under *Adjacent backlog*, not a
     lane. A fast scoped drift check:
     ```bash
     comm -3 \
       <( { issuectl --json ls --status open | jq -r '.[].slug'
            issuectl --json ls --status in-progress | jq -r '.[].slug'; } | sort -u ) \
       <( sed -n '/execution-dag:begin/,/execution-dag:end/p' TODO.md \
            | grep -oE '[a-z0-9][a-z0-9-]+' | sort -u )
     ```
   - **Validate:** every `after`/`blocked_by` target resolves to a real issue; no
     self-dep; no cycle. Surface a dangling or cyclic edge to the user — don't render a
     wrong head silently.
   - **Recompute the head-of-line** (see *Execution DAG*) and **commit `TODO.md`** if it
     changed (keep main clean).
5. **Orient the user** in one tight message: where things stand, what the pull
   brought in, **the ready frontier from the DAG** (head-of-line per lane, and what's
   blocked), and what you propose to tackle this round (fold in the `$ARGUMENTS` focus
   hint). Then proceed — don't wait for permission to *start*.

### Phase 1 — Bug intake & triage  → delegate to `/triage-bugs`

Invoke **`/triage-bugs --no-pull`** (you already pulled). It detects new
`via:telegram` bugs (lifecycle `needs-triage`), analyses the unclear ones in
read-only worktrees, **presents the product-owner briefing directly to the user**,
advances the presented bugs to `triaged`, and appends a machine-readable
`<!-- triage-return -->` block.

- If it reports no new bugs, say so and go to Phase 2.
- Otherwise **do not re-present the briefing** — `/triage-bugs` already showed it to the
  user. Read its machine-readable return block and **STOP for the user's decisions** —
  fix now / defer / not-a-bug per bug. This is the mandatory pause. Then, as the caller,
  apply each disposition (advancing the lifecycle off `triaged`):
  - defer → `issuectl label <slug> --add deferred --remove triaged`
  - not a bug → `issuectl close <slug> --status wontfix`
  - fix now → `issuectl label <slug> --remove triaged`, then carry into Phase 2's plan.
    **Do not** set `--status in-progress` here — the spinoff owns the issue lifecycle
    (`triaged` → `in-progress` on its first commit → `fixed` on merge). Setting it now
    races with the worker (per `worktree-spinoff`'s "do not call `issuectl` from the
    caller" rule).

  **Insert every fix-now bug into the execution DAG:** add one line to its lane (pick the
  lane from the bug analysis' likely-touched files; `UNLANED` if unclear). If it depends
  on another issue, record it additively — `issuectl apply` with `add_blocked_by` (never
  `set`, which can replace the list) — and mirror it as `after <slug> (needs …)`. **Commit
  the `TODO.md` edit** before moving on.

### Phase 2 — Plan the round

Combine into a work-list: the **fix-now** bugs, any feature/backlog items the user
named, and any `TODO.md` items the user wants pulled in. Then:

- **Decompose** into independent worktree units.
- **Resolve file collisions — this *is* the lane assignment.** Units that touch the
  same hot file (per the repo's AGENTS.md hot-file notes) must be **sequenced**, not run
  in parallel against that file — i.e. they share a **lane** in the DAG. Disjoint units
  run in parallel (different lanes). **If you can't tell whether two units are disjoint,
  sequence them** — a wrong guess causes merge conflicts.
- **Update the DAG for the round.** File any planned unit that isn't yet an issue (per
  repo policy) and insert it into its lane. Record real logical deps additively
  (`issuectl apply` → `add_blocked_by`) and mirror them as `after <slug> (needs …)`. Tag
  a unit that also touches a *second* lane's hot file with `collision: <file>`. **Commit
  the `TODO.md` + frontmatter edits before Phase 3 spawns anything** (so workers branch
  off committed metadata).
- **Classify each unit:** a clear, well-scoped bug/task → direct autonomous fix; a
  big or genuinely ambiguous feature → design-first.
- **Announce the plan** in one short message (which units, what's parallel vs
  sequenced). Proceed unless something is truly ambiguous.

### Phase 3 — Orchestrate (spawn worktrees; never code here)

Spawn the right worktree skill per unit. Each unit has an **explicit landing
contract** — know before launch where it lands, and **verify the actual landing from
git** before counting it toward the deploy pile:

| Unit shape | Spawn | Lands |
|---|---|---|
| Clear fix for an already-filed bug | `/worktree-spinoff --headless <slug>` (issue-driven; use the bare slug or `issuectl:<slug>`, **not** `#<slug>` — hyphenated Telegram slugs like `tg-bug-…` are not guaranteed to parse behind a `#`) | current branch (main) |
| Well-scoped autonomous task | `/worktree-spinoff --headless <task>` | current branch (main) |
| Design-first single feature | `/worktree-code <task>` (human-reviewed, foreground) or `/worktree-spinoff --headless <task>` (autonomous) | current branch (main) |

- **Autonomous spinoffs are headless** (see Standing discipline) — pass `--headless`
  on every `/worktree-spinoff`. Interactive `/worktree-code` stays foreground.
- **Requesting a review is a brief instruction, not a `--review` flag.** The
  orchestratectl `worktree-spinoff` decides review via the spinoff's *quality bar*
  (default: no review). When a unit touches **production code**, tell the spinoff in
  its task to **run `/llm-review` (+ `/assess-findings`) before merging** — that
  instruction rides in the brief; there is no `--review` passthrough flag.
- **Do not use `/worktree-bugfix <slug>`** for an already-filed bug — it treats its
  argument as a *new* free-text report and would file a duplicate. Use
  `/worktree-spinoff --headless <slug>`.
- **A multi-feature, dependency-ordered campaign is not a Phase-3 unit.** `/orchestrate`
  lands on its own integration branch (main untouched) and runs in its own window. If
  a unit is really such a campaign, **this stint becomes a hand-off**: launch
  `/orchestrate`, tell the user, and stop before Phase 4 — do not try to deploy this
  round.
- **Launch disjoint units in parallel, then wait.** Record each spawn's run id; after a
  parallel batch, block on `orchestratectl run wait <id> …` and git-verify each landing
  before counting it. **Sequence hot-file units strictly:** launch → `run wait` → verify
  the landing → *then* launch the next (so it branches off the first's landed result).
  Do not enter Phase 4 until every launched run has settled and its landing is
  git-verified. If a worker doesn't land its merge, **report it and leave main clean** —
  do not commit its work yourself. Salvage of a genuinely-dead worktree is a deliberate,
  separate manual step the user oversees, not an automatic conductor action.
- **Never write status into the DAG.** The worktree owns the issue lifecycle
  (`triaged` → `in-progress` → `fixed`); if the DAG also wrote an in-progress flag it
  would race those updates. Pick the next unit by **recomputing** the head-of-line from
  live `issuectl` status (see *Execution DAG*), not by trusting the printed `▶`. A head
  is **spawnable** only if none of its collision files (its lane's hot-file family + any
  `collision:` tag) is held by an in-progress worktree. At most an advisory
  `(spawned <run-id>)` breadcrumb next to a just-launched head — cleared at the next
  Phase-0 merge, never treated as truth.

### Phase 4 — Deploy (the conductor owns this — when the project permits)

Deploy is **conditional on project policy**, read from the repo's root `AGENTS.md`:

- **Precondition:** the pile is in `main` and **green** — run the project's green-gate
  commands first (typecheck/build/smoke). If a gate fails, **halt the deploy, report
  the failure, and spawn a fix worktree** for it; do not deploy red.
- **Deploy autonomy:** if the project grants deploy-without-asking (typical in an
  active test-cycle, where deploy targets a **test/staging server**, not production),
  deploy directly. If it requires confirmation, targets production, or `AGENTS.md` is
  **silent** on autonomy, ask once and suggest documenting a deploy-autonomy policy.
- Run **one** deploy with the project's exact command from `AGENTS.md` (including any
  required env export, flags, and post-deploy steps). Never parallel. Then **verify
  live** (the project's health check / smoke) and report the outcome.

If the project has no deploy step for a stint (e.g. changes land on main and a human
promotes later), skip this phase and say so.

### Phase 5 — Report to the user  → `/worktree-status`

The coding happened in detached worktrees, so this conversation doesn't yet know what
landed. **First gather the round's durable facts into the conversation:** the
commits that landed on main (`git log --oneline`), the issues that closed and their
analyses, and anything the workers wrote back (e.g. bug-analysis notes, worker
reports). State those verified facts in chat. **Then** invoke **`/worktree-status`**,
which formats what's now in context into the product-owner snapshot: Summary · Ready
to test · Decisions needed · Discussion points · Spin-offs. Your reactions seed the
*next* stint.

### Phase 6 — Absorb the user's feedback

The `/worktree-status` snapshot hands the user things to act on — items to test,
discussion points, spin-off calls. This is where they react, and their reactions
decide what happens next.

- **Light feedback** (a handful of small asks) → fold it into a quick pass: each fix
  as its own worktree (Phases 2–4 in miniature). Still no coding in this session —
  every change goes through a worktree.
- **Heavy feedback** (a lot comes back) → don't try to carry it in this session's
  context. **Land it durably first** — update the affected **issues**,
  **documentation**, and **`TODO.md`** so nothing is lost — *then* continue to the
  handoff.

**Keep the DAG current before any mini-round.** If feedback files new issues or changes a
dependency, **insert them into the DAG and commit** (same edit as Phase 1/2) *before* a
Phase-6 mini-round consults it — a mini-round that sequences against a stale DAG can
mis-order or miss an issue. If you capture feedback durably without a mini-round, still do
the insert so Phase 7 is already accurate.

Once the feedback is absorbed (acted on via worktrees, or captured durably), move on.

### Phase 7 — Handoff / wrap-up (propose; run only on request)

A stint typically fills this session's context after ~one round. When you notice that
(or the user asks), **propose** the handoff. On the user's go, and only then:

1. **Update the `TODO.md` handoff block** (`## 🔄 Continue here` / `ALOITA TÄSTÄ`) so
   a fresh agent can resume from `jatketaan @TODO.md` — this is an inline stint action,
   not a separate skill. **In the same edit, merge the execution DAG one last time**
   (Phase-0 merge: drop landed issues, add any still-open ones, refresh the date stamp,
   set the `GLOBAL HEAD-OF-LINE`) so the next resume opens onto an accurate graph.
2. **Commit the `TODO.md` handoff + DAG update immediately** — commit it on its own
   (`git add TODO.md && git commit`) *before* the next step, so it doesn't get folded
   into `/wrap-up`'s mixed commit or left dangling.
3. `/wrap-up` — it will *present proposed* `AGENTS.md`/issue/preference changes and
   ask before writing; don't assume it committed unless it reports saved changes.
4. If the project's AGENTS.md/TODO declares a **test-account reset preference**, do it
   or remind the user.

Do not auto-run these — propose and wait.

## Execution DAG (the convention)

The DAG is the round's **scheduling plan**, living in a delimited block in `TODO.md`. It
is generic: this skill defines the *notation and rules*; the actual lanes and hot files
are **project facts** in the repo's `AGENTS.md` / `TODO.md`, never hardcoded here.

**What it owns vs. what it reads.** The DAG is authoritative for the *plan* — each issue's
lane, the order within a lane, and cross-lane `collision:` tags. It stores **no status**;
`issuectl` is authoritative for status, read on demand. No fact lives in both, so they
cannot drift. You **merge** the DAG (Phase 0/7) — never regenerate it from scratch, which
would force re-deriving the hot-file collision matrix and risk hallucinating or dropping a
`collision:` edge.

**Lanes = hot-file families.** Two issues whose fixes touch the same hot file share a lane
and are sequenced (≤1 live worktree per lane at a time). Disjoint issues sit in different
lanes and run in parallel. Issues touching no hot file go `UNLANED`. Deferred / not-in-plan
issues live under `## Adjacent backlog`, **not** a lane.

**Canonical block** (delimited so tooling and the `comm -3` check parse only the nodes):

````markdown
## Execution DAG (<YYYY-MM-DD>)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge at Phase 0/7 (drop landed, add active, keep existing order).
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
UNLANED — no shared hot files, run anytime:
    <slug-x>, <slug-y>
```
<!-- execution-dag:end -->
````

**Head-of-line (compute on read, never trust the printed `▶`):**

- An issue is **eligible** iff its `issuectl` status is `open`/`in-progress` (in the active
  set), it is **not** `deferred`-labelled, it is **not already** `in-progress` (a live
  worktree has it), and every `blocked_by` target is **delivered** — `status ∈ {fixed,
  done}`. A target that is `wontfix` / `obsolete` / `cannot-reproduce` does **not** satisfy
  the dependency (the code was never built) — the dependent stays blocked; flag it to the
  user. Follow a `duplicate` to its canonical issue.
- `head(lane)` = the first eligible issue in that lane's order.
- A head is **spawnable** iff none of its collision files (its lane's hot-file family + any
  `collision:` tag) is held by an in-progress worktree.
- `GLOBAL HEAD-OF-LINE` = pick among spawnable heads: an explicit handoff "start here"
  first, else highest `issuectl` priority, else earliest in lane order, else slug order.

**Slugs, not positional codes.** Reference issues by slug everywhere (no `A1`/`B5`);
positional numbers churn on every insert. Lane letters are just coarse group labels.

Requires the repo to keep the delimited `## Execution DAG` section in `TODO.md` and a
hot-file list in `AGENTS.md` (see *Project prerequisites*).

## Project prerequisites (what the repo's AGENTS.md should provide)

This skill is generic, so it relies on the repo documenting its own operating facts.
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

## Non-goals

- **Not a worktree**, and does not create one directly — it delegates to the
  `/worktree-*` family.
- **Does not write code** in this session — every change goes through a worktree.
- **Not for a single one-off coding task** — that's `/worktree` (router).
- **Not for bare** triage / status / deploy / handoff-block update — those are
  `/triage-bugs`, `/worktree-status`, the project deploy command, and an inline
  `TODO.md` handoff-block edit.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md.
- Does not decide fix/defer/not-a-bug — the user does (Phase 1).
