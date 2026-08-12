---
name: stint-start
description: "Run one round of a work-session (työrupeama, 'stint') as the ORCHESTRATOR the user talks to. The round engine: orient (pull, read operating policy, ground-truth from git, merge the execution DAG) → plan → spawn worktrees that do the coding (never codes in this session) → own the single deploy when the project permits → report to the user in product-owner language via `/worktree-status` → absorb feedback (a feedback mini-round is just a re-run of this skill). Use when the user says 'aloitetaan rupeama', 'jatketaan @TODO.md', 'start a work session', 'let's do a round', 'do another round', or invokes bare `/stint-start`. Maximally autonomous — resume straight from the handoff-prepared agenda (the `## 🔄 Continue here` block + DAG, with approved intake items already folded in) and just go, asking nothing it can read or decide for itself. Generic across projects — reads all project specifics from the repo's own AGENTS.md/TODO.md. NOT a worktree itself; NOT for a single one-off coding task (use `/worktree`); NOT for bug intake/triage (surfaced and folded into the agenda at `/stint-handoff`, so it enters here as already-planned work); NOT for the terminal handoff/wrap-up (that is `/stint-handoff`)."
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Stint-start — the work-session round engine

You are the **orchestrator the user talks to**. A stint (työrupeama) is one round of
the standing loop: *pull → plan → spawn worktrees that do the coding → one deploy →
report to the user → absorb feedback.* You conduct; **you do not write feature code in
this session.** The actual implementation happens in worktrees you spawn, so this
conversation's context stays free for orchestration and for talking to the user.

Run this skill **every round**. A feedback mini-round (Phase 5) is just a fresh re-run of
this same skill — a full pass from Phase 0 — on the smaller work-list; there is no
separate mini-round logic. When the session is done and the user asks to hand off, that
terminal wrap is a **different** skill: **`/stint-handoff`** (update the `TODO.md` handoff
block + final DAG merge, then `/wrap-up`).

This skill is **generic**. Every project-specific fact — the deploy command, whether
you may deploy without asking, the green-gate commands, hot files, the test-account
reset preference — is read from the **repo's own `AGENTS.md` and `TODO.md`**. If a
needed fact is missing, **prefer resolving it yourself**: read it from `AGENTS.md` /
`TODO.md` / git, or log a best-judgment decision and proceed (bold first, ask later),
and note it should be documented. Ask the user only when the fact is genuinely
unresolvable *and* blocking (see *Autonomy*). It assumes this toolchain —
**`issuectl`** for issues and the **`/worktree-*`** family (`orchestratectl` underneath)
for workers — and is a layer on top of them. Read `orchestratectl-overview` and
`worktree-spinoff` before your first spawn.

The Execution-DAG convention, the operating-policy facts to read, and the project
prerequisites live in the shared reference **[`AGENTS-EXECUTION-DAG.md`](AGENTS-EXECUTION-DAG.md)**
(installed alongside this skill). **Open and read it before Phase 0** — Claude Code loads
only this `SKILL.md`, so the linked file is not in context until you open it, and the
phases below reference its merge algorithm rather than repeating it. If the file is
missing or unreadable, stop and report an incomplete skill install rather than improvising
the DAG merge from memory.

> **Intake is surfaced and folded in at handoff, not here.** This skill does **not**
> triage incoming bug reports. New intake items are detected, listed to the human, and
> folded into the next stint's agenda by **`/stint-handoff`** (its intake-check step) —
> so by the time `/stint-start` runs, the approved items are already **normal planned
> work** in the `## 🔄 Continue here` block + the execution DAG. Consume them from there;
> do **not** expect the user to hand you bug slugs mid-start, and do **not** run triage.
> `stint-start` keeps only the plain Phase-0 `git pull`.

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
  status. You **merge** it (drop only terminal issues, add active/non-terminal ones, keep
  the existing plan) — you never regenerate it from scratch. The head-of-line ("what's next") is **computed
  on read** by joining the DAG's lane order with live `issuectl` status; the printed
  `▶` marker is only a snapshot. Full convention in the shared
  [`AGENTS-EXECUTION-DAG.md`](AGENTS-EXECUTION-DAG.md). Editing the DAG is orchestration,
  not product code — do it in this session.
- **Autonomous spinoffs run headless.** Every self-merging spinoff you spawn directly
  (`/worktree-spinoff`) passes `--headless`, so the round's workers land in the detached
  `headless` tmux session instead of cluttering the user's window list; attach with
  `tmux attach -t headless` only when curious. Auto-cleanup still closes each window on
  terminal. Only interactive `/worktree-code` — which the user actively drives and
  reviews — stays foreground.
- **Sync with `run wait`; trust the CLI's `landed` flag for the landing.** A spinoff runs
  **asynchronously** — its spawn call returns immediately. Record every returned run id and
  block on `orchestratectl run wait <run-id> …` to know the workers have *settled* before
  you sequence the next unit or enter Phase 3. But do **not** trust run *status* as proof
  the work landed: `orchestratectl run show` can report a false `failed` / `pending` even
  when the worker committed **and** merged. **To confirm a landing, read the CLI's
  `landed` boolean** (surfaced by both `run wait` and `run show`). It is git-verified against
  the *current* target tip — patch-id equivalence plus an ancestry safety net — so it stays
  correct after you rebase local `main`. The companion `landed_method` tells you the evidence:
  `git-verified` (git decided), `report-marker` (git could not run — branch already torn
  down — so the durable `run merge` marker decided), or `unverified`. Settled ≠ landed; the
  `landed` flag is the landed signal.
  - **⚠️ Do NOT git-verify with `git merge-base --is-ancestor <worker-branch> <target>`.**
    In a busy repo you rebase local `main` onto `origin/main` every round; that **replays
    the worker's merge under a new hash** while the worker **branch ref stays at its
    pre-rebase hash**, so `--is-ancestor` returns a **false "not landed"** even though the
    content is fully merged. This trap fired twice in one real stint and nearly triggered a
    destructive re-spawn / hand-salvage of already-merged work. The CLI `landed` flag exists
    precisely to replace this check.
  - **`landed: false` is not always "not landed."** If `landed_method` is `git-verified`,
    trust it — git positively found unlanded work (or genuine absence). If it is `unverified`,
    the CLI *could not confirm* (missing inputs, transient git error) — do **not** auto-respawn
    or salvage on that alone; verify by content first.
  - If you must double-check by hand, verify by **content on the actual target the run merged
    into** (usually local `main`, or the integration branch for an orchestrated child) — never
    by the worker branch ref. Check for the expected files/symbols on that target, or the
    intended diff (`git diff <base>..<target> -- <paths>`); a `git log … | grep <subject>` is
    weak (subjects change under rebase/squash and can collide). If `landed` and your manual
    check disagree, treat it as a reconciliation point — block and investigate rather than
    auto-deploying or auto-salvaging.
- **One deploy at a time.** Never parallel deploys.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).

## Autonomy

Autonomy is **maximal — just go**. `/stint-handoff` has already left the start fully
prepared (the `## 🔄 Continue here` block + the execution DAG, with any approved intake
items already folded in), so **trust that prepared state and start executing** — do not
re-derive or re-confirm the plan with the user, and do not expect them to hand you slugs.
Run orienting → planning → orchestration → deploy → report autonomously; narrate state
changes and decisions, not internal deliberation. When a fact is missing, prefer reading
it or logging a best-judgment decision and proceeding, rather than asking. Pause only
for: (a) a genuine fork the handoff could **not** have resolved and where a wrong call
would be costly to undo, (b) deploy go/no-go **if** the project has not pre-authorised
deploys (see Phase 3), (c) the transition to handoff/wrap-up, which is a separate skill
(`/stint-handoff`) you propose and run only on the user's go. The product-owner status
report (Phase 4) is **output, not a question** — always deliver it.

"Prefer best-judgment and proceed" governs **reversible scheduling / implementation-detail**
choices only. It never overrides these hard stops — halt or pause, don't guess:
- a **missing green-gate or migration command** for work that needs it (don't skip the gate);
- a **deploy target/autonomy** that `AGENTS.md` leaves ambiguous (Phase 3's rule wins);
- an **ambiguous file collision** — sequence the units, never guess parallel (Phase 1);
- the **landing-verification** warning — a `landed`/manual-check disagreement blocks, and
  `landed_method: unverified` is never grounds to auto-respawn or auto-deploy;
- **cold start** with no prepared plan (Phase 1) — a single planning pass, not invention.

## Phases

### Phase 0 — Orient (bootstrap)

1. **Pull.** `git pull --ff-only` in the repo. If it can't fast-forward, stop and
   report; do not force.
2. **Read the operating policy** from the repo's root `AGENTS.md` and `CLAUDE.md`, and
   the `TODO.md` handoff block (`## 🔄 Continue here` / `ALOITA TÄSTÄ`). The exact facts
   to gather — deploy command + autonomy, deploy target, green-gate commands, live-version
   check, hot-file list, migration rules, test-account reset preference — are listed in the
   shared [`AGENTS-EXECUTION-DAG.md`](AGENTS-EXECUTION-DAG.md) § *Reading the operating
   policy*.
3. **Establish ground truth from git**, not from the handoff's claims: is `main` ==
   what's deployed (use the project's live-version check)? Merged-but-undeployed
   work? Half-finished worktree branches? (`git log --oneline`, compare against the
   handoff's stated prod image/version.)
4. **Merge the execution DAG against reality** (do this *first*, before orienting —
   otherwise you orient off stale data). This is a stateful **merge**, not a rewrite:
   preserve the existing lane assignment, order, and `collision:` tags; only reconcile
   the *set* of issues. The full merge procedure — active-set fetch, drop/add rules, the
   `comm -3` drift check, edge validation, and head recompute — is in the shared
   [`AGENTS-EXECUTION-DAG.md`](AGENTS-EXECUTION-DAG.md) § *Execution DAG (the convention)*.
   Then **commit** the changed files (`TODO.md` plus any issue files `issuectl`
   rewrote — name the exact paths, not `git add -A`) so main is clean before Phase 1.
5. **Orient the user** in one tight message: where things stand, what the pull
   brought in, **the ready frontier from the DAG** (head-of-line per lane, and what's
   blocked), and what you propose to tackle this round (fold in the `$ARGUMENTS` focus
   hint). Then proceed — don't wait for permission to *start*.

### Phase 1 — Plan the round

The work-list is **already prepared** — take it from the handoff-built `## 🔄 Continue
here` block + the execution DAG (which includes any intake items the human acked and
folded in at `/stint-handoff`). Fold in the `$ARGUMENTS` focus hint and any items the
user explicitly names this round, but do **not** re-ask the user to confirm the plan the
handoff already supplied.

- **Cold start (no prepared state).** "Trust the prepared state" assumes there *is* one.
  If the `## 🔄 Continue here` block or the DAG is **missing or empty** (a fresh clone,
  the repo's first run, or a stint that never reached handoff), do **not** invent a plan
  and do **not** silently treat the entire open backlog as this round's agenda. Bootstrap
  it: the Phase-0 DAG merge already built the active set from live `issuectl` status, so
  orient from that computed ready frontier, state plainly that no prepared narrative
  exists, and — since there is no human-vetted plan to trust — do a single planning pass
  with the user before spawning. This is a legitimate pause (a genuine fork the handoff
  could not have resolved), not a violation of the autonomy posture.
- A **deliberately empty** prepared agenda (handoff ran, acked nothing, no active work) is
  not a cold start — report "no ready work" and skip spawn/deploy rather than manufacturing
  units from the backlog.

Then:

- **Decompose** into independent worktree units.
- **Resolve file collisions — this *is* the lane assignment.** Units that touch the
  same hot file (per the repo's AGENTS.md hot-file notes) must be **sequenced**, not run
  in parallel against that file — i.e. they share a **lane** in the DAG. Disjoint units
  run in parallel (different lanes). **If you can't tell whether two units are disjoint,
  sequence them** — a wrong guess causes merge conflicts.
- **Update the DAG for the round.** File any planned unit that isn't yet an issue (per
  repo policy) and insert it into its lane. Record real logical deps additively
  (`issuectl apply` → `add_blocked_by`, only on issues not yet `in-progress`) and mirror
  them as `after <slug> (needs …)`. Tag a unit that also touches a *second* lane's hot file
  with `collision: <file>`. **Only mutate frontmatter of an issue that is not yet
  `in-progress`** — once a worktree owns it, its issue file is worker-owned (calling
  `issuectl` on it races the worker per `worktree-spinoff`); note the intended dep in the
  DAG and reconcile after it lands. **Commit `TODO.md` plus every issue file `issuectl`
  rewrote (name the paths, not `git add -A`) before Phase 2 spawns anything** — verify a
  clean tree so workers branch off committed metadata.
- **Classify each unit:** a clear, well-scoped bug/task → direct autonomous fix; a
  big or genuinely ambiguous feature → design-first.
- **Announce the plan** in one short message (which units, what's parallel vs
  sequenced). Proceed unless something is truly ambiguous.

### Phase 2 — Orchestrate (spawn worktrees; never code here)

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
- **A multi-feature, dependency-ordered campaign is not a Phase-2 unit.** `/orchestrate`
  lands on its own integration branch (main untouched) and runs in its own window. If
  a unit is really such a campaign, **this stint becomes a hand-off**: launch
  `/orchestrate`, tell the user, and stop before Phase 3 — do not try to deploy this
  round.
- **Launch disjoint units in parallel, then wait.** Record each spawn's run id; after a
  parallel batch, block on `orchestratectl run wait <id> …` and confirm each landing via the
  CLI's `landed` flag before counting it (NOT `merge-base --is-ancestor` — see the landing
  warning above). **Sequence hot-file units strictly:** launch → `run wait` → confirm
  `landed` → *then* launch the next (so it branches off the first's landed result).
  Do not enter Phase 3 until every launched run has settled and its `landed` flag is true.
  If a worker doesn't land its merge, **report it and leave main clean** —
  do not commit its work yourself. Salvage of a genuinely-dead worktree is a deliberate,
  separate manual step the user oversees, not an automatic conductor action.
- **Recoverable worker death → retry-with-harvest, never hand-merge.** A worker can die
  (`run wait` / `run show` report an `agent-died` **failed** run) after it committed clean,
  mergeable work but before it called `run merge`. The supervisor preserves that branch and
  stamps a `recoverable_work` block onto the failed report, which `run wait` surfaces per
  run (`recoverable=<n> unmerged commit(s) merge cleanly on <branch>` when
  `recoverable: true`, `merges_cleanly: true`, `unmerged_commits > 0`). When you see that:
  - **Do NOT hand-merge the preserved branch from this session.** Those commits are
    *unreviewed* — no green gate, no `/llm-review` — and merging them yourself both breaks
    "never commit a worker's work for it" and lands unvetted code. Cherry-picking or
    `git merge`-ing it here is the wrong move.
  - **Re-spawn a fresh worktree pointed at the preserved branch** — a `/worktree-spinoff
    --headless` for the *same issue* whose brief names the preserved branch and instructs
    it to: review the stranded commits, **adopt** them (cherry-pick / re-apply onto a fresh
    branch off current main), complete the green gate, run `/llm-review` (+
    `/assess-findings`) for production code, and merge. This is **retry-with-harvest**: a
    fresh reviewing agent finishes the dead worker's work — **not** a hand-merge, and
    **not** a base-agent swap (the model/harness is fine; the process just died).
  - **Deaths are transient — the retry usually lands.** Don't infer a systemic problem from
    one `agent-died`; re-spawn and let it run. And a **long** run is not a hang:
    heavy-LLM units (design-first, multi-round review) legitimately run **54–96 min**, so
    keep waiting on `run wait` rather than assuming a second death.
  - **After the harvest lands, the superseded preserved branch/worktree is an orphan.**
    Once the retry has git-verified-merged the same work, the original dead worker's branch
    and worktree are safe to remove — but that removal is a **deliberate, human-overseen
    cleanup**, not an automatic conductor action (the intended `run salvage` command will
    fold this in; until it ships, retry-with-harvest is the manual stand-in).
- **Never write status into the DAG** — not even a spawn breadcrumb (that would leave
  `TODO.md` dirty across the phase and pollute the drift check). The worktree owns the
  issue status lifecycle (`in-progress` → terminal `fixed`/`done`); any intake lifecycle
  label was already resolved at handoff (an admitted item no longer carries
  `needs-triage`). If the DAG also wrote status it would race those updates. Track which units you've launched this round in **conductor
  memory** (the recorded run ids), not in the file. Pick the next unit by **recomputing**
  the head-of-line from live `issuectl` status (see the shared
  [`AGENTS-EXECUTION-DAG.md`](AGENTS-EXECUTION-DAG.md)), never from the printed `▶`.
- **Reserve collision files at launch, not at first commit.** A spawned worker does not
  flip its issue to `in-progress` until its first commit, so keying spawn-eligibility off
  `issuectl` status alone leaves a window where two heads sharing a hot file both look
  free. Treat every **launched-but-unsettled run this round as already holding its
  collision files** (its lane's hot-file family + any `collision:` tag) **and its issue**.
  A head is spawnable only if its collision files intersect no unsettled run's and no
  unsettled run already covers its issue — otherwise sequence it (this is why same-lane
  units already go launch → `run wait` → verify → next).

### Phase 3 — Deploy (the conductor owns this — when the project permits)

Deploy is **conditional on project policy**, read from the repo's root `AGENTS.md`:

- **Precondition:** the pile is in `main` and **green** — run the project's green-gate
  commands first (typecheck/build/smoke). If a gate fails, **halt the deploy, report
  the failure, and spawn a fix worktree** for it (then wait, git-verify its landing, and
  re-run the full green gate before reconsidering the deploy); do not deploy red.
- **Deploy autonomy:** if the project grants deploy-without-asking (typical in an
  active test-cycle, where deploy targets a **test/staging server**, not production),
  deploy directly. If it requires confirmation, targets production, or `AGENTS.md` is
  **silent** on autonomy, ask once and suggest documenting a deploy-autonomy policy.
- Run **one** deploy with the project's exact command from `AGENTS.md` (including any
  required env export, flags, and post-deploy steps). Never parallel. Then **verify
  live** (the project's health check / smoke) and report the outcome.

If the project has no deploy step for a stint (e.g. changes land on main and a human
promotes later), skip this phase and say so.

### Phase 4 — Report to the user  → `/worktree-status`

The coding happened in detached worktrees, so this conversation doesn't yet know what
landed. **First gather the round's durable facts into the conversation:** the
commits that landed on main (`git log --oneline`), the issues that closed and their
analyses, and anything the workers wrote back (e.g. bug-analysis notes, worker
reports). State those verified facts in chat. **Then** invoke **`/worktree-status`**,
which formats what's now in context into the product-owner snapshot: Summary · Ready
to test · Decisions needed · Discussion points · Spin-offs. Your reactions seed the
*next* round.

### Phase 5 — Absorb the user's feedback

The `/worktree-status` snapshot hands the user things to act on — items to test,
discussion points, spin-off calls. This is where they react, and their reactions
decide what happens next.

- **Light feedback** (a handful of small asks) → **re-run this whole skill** on the
  smaller work-list: a fresh `stint-start` pass from Phase 0 (pull, ground-truth, DAG
  merge, plan, spawn, deploy, report), just with fewer units. There is no separate
  mini-round logic and no "phases in miniature" — a feedback round *is* an ordinary
  `stint-start` round. Still no coding in this session — every change goes through a
  worktree.
- **Heavy feedback** (a lot comes back) → don't try to carry it in this session's
  context. **Land it durably first** — update the affected **issues**,
  **documentation**, and **`TODO.md`** so nothing is lost — *then* move to the handoff.

**Keep the DAG current before any re-run.** If feedback files new issues or changes a
dependency, **insert them into the DAG and commit** (same edit as Phase 1) *before* a
feedback re-run consults it — a re-run that sequences against a stale DAG can mis-order or
miss an issue. If you capture feedback durably without a re-run, still do the insert so the
eventual handoff (`/stint-handoff`) opens onto an accurate graph.

Once the feedback is absorbed (acted on via worktrees, or captured durably), the round is
done. When the session's context is filling or the user asks to wrap up, **propose
`/stint-handoff`** — the terminal wrap is a separate skill, not part of this one.

## Non-goals

- **Not a worktree**, and does not create one directly — it delegates to the
  `/worktree-*` family.
- **Does not write code** in this session — every change goes through a worktree.
- **Not for a single one-off coding task** — that's `/worktree` (router).
- **Not for bug intake / triage** — the round engine does not run it. New intake items
  are surfaced and folded into the agenda at `/stint-handoff` (its intake-check step), so
  they enter Phase 1 as already-planned work-list / DAG items, not as slugs the user hands
  you mid-start.
- **Not the terminal handoff/wrap-up** — that's `/stint-handoff` (update the `TODO.md`
  handoff block + final DAG merge, then `/wrap-up`).
- **Not for bare** status / deploy — those are `/worktree-status` and the project deploy
  command.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md.
