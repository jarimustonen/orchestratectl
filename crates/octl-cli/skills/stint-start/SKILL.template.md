---
name: stint-start
description: "Run one round of a work-session (työrupeama, 'stint') as the ORCHESTRATOR the user talks to. The round engine: orient (pull, read operating policy, ground-truth from git, read the issuectl scheduling DAG) → plan → spawn worktrees that do the coding (never codes in this session) → own the single deploy when the project permits → report to the user in product-owner language → absorb feedback. Use when the user says 'aloitetaan rupeama', 'jatketaan @TODO.md', 'start a work session', 'let's do a round', 'do another round', or invokes bare `/stint-start`. Maximally autonomous: resume from the handoff narrative and the live issuectl DAG. Generic across projects: reads all specifics from the repo's own AGENTS.md/TODO.md and issue metadata. NOT a worktree itself; NOT a single one-off coding task (use `/worktree`); NOT the terminal handoff/wrap-up (that is `/stint-handoff`)."
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
narrative, verify the live issue schedule, then `/wrap-up`).

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

Scheduling requires **`issuectl dag --json`** with `--reservations`. It is the sole
source for lane order, dependency state, collision tokens, computed heads, and
spawnability. `TODO.md` is only the handoff narrative: never infer or maintain scheduling
structure there. If issuectl, the DAG command, its required JSON fields, or reservation
support is unavailable, stop and report an unmigrated or incompatible project. Never
fall back to a prose schedule.

## Standing discipline (holds across every phase)

- **Orchestrate, don't code.** Every code change — feature, bugfix, or one-line
  trivia — goes through a worktree, never this session. If you catch yourself about
  to edit product code, stop and spawn a worktree. (Editing `TODO.md`, `AGENTS.md`,
  and issue files as part of orchestration is fine — that's not product code.)
- **Keep main clean; worktrees own their commits.** Parallel worktrees branch off
  main's current state. Never leave main modified-but-uncommitted across a phase, and
  **never commit a worker's in-progress work for it** — each worktree commits its own
  changes. If a worker didn't land, report it; do not rescue it by committing on main.
- **Read scheduling from issuectl.** After Phase 0 reconstructs holds, every work-selection
  read must use `issuectl dag --json --reservations "$reservations"`. A bootstrap read
  with `[]` may resolve hold metadata but must never drive spawning until run enumeration
  proves the hold set is actually empty. Read `.data.lanes[]` for ordered issues and
  computed heads, each issue's dependency, collision, and `spawnable` fields,
  `.data.unscheduled`, and numeric `.data.spawnable_heads`. An `in-progress` head remains
  resumable; reservations, not status, prevent duplicate work. Never copy this graph into
  `TODO.md` or recompute what the command already derives. A command failure, malformed
  envelope, missing blocker, self-dependency, or cycle makes the schedule invalid: stop
  before selecting or spawning work. A dependency is satisfied only by a delivering
  terminal status. In the default schema, `fixed` and `done` deliver; `wontfix`,
  `obsolete`, `cannot-reproduce`, and `duplicate` do not. Use the consuming project's
  equivalents if it customizes statuses. An entry in `.data.unscheduled` has no executable
  lane. Assign a conservative lane only when you intend to run that issue this round, or
  use the reserved `unlaned` lane only when it is confirmed parallel-safe. Leave deferred
  or out-of-plan entries unscheduled and report them.
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
- **Read every settled worker's report before planning the next wave.** The persisted
  projection field is `last_report`; use the read surface rather than opening run files.
  For a single-worker run, `run show` has `data.report`. Multi-node runs must
  inspect each node:

  ```bash
  # skill-example-ci: skip (the parser validates CLI argv, not shell pipelines)
  orchestratectl run show "$run_id" --output json | jq '.data.report'
  # Node-level projection-compatible probe:
  # skill-example-ci: skip (the parser validates CLI argv, not shell pipelines)
  orchestratectl node show "$run_id" n-0001 --output json |
    jq '.data.report // .data.last_report'
  ```

  `run wait` can return several runs, so its envelope is `data.runs[]`, not
  `data.<field>`: use `jq '.data.runs[] | {run_id, status, summary}'` and never
  `.data.status`. It folds in `summary`; use `run show`/`node show` for the full
  `discussion_items`, `spinoff_proposals`, and `wrap_up_recommendations` needed
  to sequence later lane work.
- **One deploy at a time.** Never parallel deploys.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).

## Autonomy

Autonomy is **maximal: just go**. `/stint-handoff` has left a prepared narrative and
issuectl carries the live schedule, so **trust those sources and start executing**. Do not
re-derive or re-confirm the plan with the user.
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
   the `TODO.md` handoff block (`## 🔄 Continue here` / `ALOITA TÄSTÄ`). Gather the deploy
   command and autonomy, deploy target, green-gate commands, live-version check, hot-file
   guidance, migration rules, and test-account reset preference. Hot-file guidance must
   name shared files or file families precisely enough to map issue work into serial lanes
   and cross-lane collision tokens. Deploy policy must state the exact command, target,
   and autonomy; the live-version check and green gate must be executable commands. If a
   required fact is missing, resolve it from project docs or git where possible; only ask
   when it is both unresolvable and blocking, and recommend documenting it.
3. **Establish ground truth from git**, not from the handoff's claims: is `main` equal to
   what's deployed (use the project's live-version check)? Is there merged-but-undeployed
   work or a half-finished worktree branch? Compare `git log --oneline` with the handoff's
   stated live image or version.
4. **Read the live schedule and reconstruct holds.** Verify the prerequisite first with
   `issuectl dag --help` and verify scheduling mutations with `issuectl update --help`.
   Set `reservations='[]'` and run the command once to obtain the
   issue-to-lane/collision mapping:

   ```bash
   reservations='[]'
   issuectl dag --json --reservations "$reservations"
   ```

   This bootstrap read is also the schema gate: confirm `.data.lanes[]`,
   `.data.unscheduled`, and numeric `.data.spawnable_heads` exist, and lane issues carry
   `collision` arrays plus Boolean `spawnable` values. If any field is absent, stop as
   incompatible rather than guessing. Inspect `orchestratectl run list --output json` and
   the relevant `run show` records for
   every live or resumable run. Map each run's issue slug through the first DAG response,
   then replace `reservations` with the exact issuectl hold-array shape, one object per
   run: `[{"lane":"backend","collision":["path/to/hot-file"]}]`. Two live runs are
   two array objects, even when their lanes match. Include the issue's lane and its
   complete `.collision` array; collision tokens are exact opaque strings copied from
   issue metadata, not inferred paths or lane names. Re-run the command with that payload;
   only this reservation-aware response may drive spawning. If there are no holds, the
   second read still uses `[]`. Validate assembled JSON before use. If a run cannot be
   mapped to its issue and complete hold, inspect it with `run show` and resolve any open
   awaiting-input request, retry-with-harvest case, cancellation, or other ownership state
   first. A run relinquishes ownership only after it lands or a terminal cancel/abandon
   path confirms that no preserved worktree, branch, or resumable work remains. Do not
   spawn while ownership remains ambiguous. If the command fails, its JSON
   is malformed, or the graph is invalid, stop; never retry without reservations or patch
   around the failure in `TODO.md`.
5. **Orient the user** in one tight message: where things stand, what the pull brought
   in, the computed head per lane, what is blocked or unscheduled, and what you propose to
   tackle this round (fold in the `$ARGUMENTS` focus hint). Then proceed without waiting
   for permission to start.

### Phase 1 — Plan the round

The work-list is **already prepared**: use the `## 🔄 Continue here` narrative for intent
and a reservation-aware DAG read for the current executable frontier. Shell variables do
not survive between tool calls: for every read, assign the current JSON and invoke issuectl
in the same shell command, or pass a recorded reservation-file path. Never emit
`--reservations ""`; pass `'[]'` explicitly when run enumeration proves there are no
holds. Fold in the `$ARGUMENTS` focus hint and any items the user explicitly names this
round, but do **not** re-ask the user to confirm the plan the handoff already supplied.

- **Cold start (no prepared narrative).** If the handoff block is missing or empty, do not
  invent a plan and do not silently treat the entire open backlog as this round's agenda.
  Orient from issuectl's computed frontier, state plainly that no prepared narrative
  exists, and do one planning pass with the user before spawning. This is a legitimate
  pause because no human-vetted intent exists.
- A **deliberately empty** prepared agenda (handoff ran, acked nothing, no active work) is
  not a cold start — report "no ready work" and skip spawn/deploy rather than manufacturing
  units from the backlog.

Then:

- **Decompose** into independent worktree units.
- **Resolve file collisions through issue metadata.** Units that touch the same hot file
  must be sequenced in one lane; disjoint units can use different lanes. Shared resources
  across lanes belong in each issue's `collision` list. If disjointness is unclear,
  sequence the units.
- **Update issue metadata for the round.** File any planned unit that is not yet an issue.
  Use the validated CLI operations, for example `issuectl update <slug> --lane <lane>
  --lane-seq <n> --add-blocked-by <blocker> --add-collision <token> --json`; repeat the
  additive flags as needed rather than replacing lists. Never edit a second scheduling
  copy in prose. Only mutate an issue that is not yet owned by a live worktree because a
  concurrent issuectl write races that worker; otherwise defer the metadata change until
  it lands. Deferred work remains represented by issue status/metadata and is not pulled
  into the round unless the prepared intent names it. Commit every rewritten issue path
  by exact name, never `git add -A`, before Phase 2 and verify a clean tree. Re-run
  `issuectl dag --json --reservations "$reservations"` and use its computed order,
  blockers, heads, and spawnability as the plan.
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
| Clear fix for an already-filed bug | `/worktree-spinoff --headless <slug>` (issue-driven; use the bare slug or `issuectl:<slug>`, **not** `#<slug>` because hyphenated slugs are not guaranteed to parse behind a `#`) | current branch (main) |
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
- **Reserve at launch, not at first commit.** Immediately after spawning, add that run's
  issue slug, run id, lane, and complete collision-token list to conductor memory; the
  JSON passed to issuectl contains each hold's `lane` and `collision` fields. Before every
  subsequent pick, materialize the complete current hold array and pass it in the same
  shell invocation, for example `reservations='[...]'; issuectl dag --json --reservations
  "$reservations"`. Launch only a lane issue whose
  own `spawnable` field is true, and separately exclude every slug already held in
  conductor memory because the reservation schema carries resource tokens, not issue
  identity. issuectl treats `unlaned` as parallel-safe, so a hold carrying that lane does
  not reserve other `unlaned` issues; collision tokens and the separate slug guard still
  apply. Do not release a hold merely because `run wait` returned: awaiting-input or
  recoverable runs can settle while retaining resumable work. Resolve awaiting-input
  requests before selecting again. Release a hold only after `run show` confirms the run
  has landed, or an explicit cancel/abandon path confirms no preserved or resumable
  ownership remains. This closes both the pre-commit and
  attention-required windows. Do not write spawn breadcrumbs or run status to `TODO.md`;
  issue status belongs to the worker and live holds belong to conductor memory.

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
  smaller work-list: a fresh `stint-start` pass from Phase 0 (pull, ground-truth,
  issuectl schedule read, plan, spawn, deploy, report), just with fewer units. There is no separate
  mini-round logic and no "phases in miniature" — a feedback round *is* an ordinary
  `stint-start` round. Still no coding in this session — every change goes through a
  worktree.
- **Heavy feedback** (a lot comes back) → don't try to carry it in this session's
  context. **Land it durably first** — update the affected **issues**,
  **documentation**, and **`TODO.md`** so nothing is lost — *then* move to the handoff.

**Keep issue scheduling current before any re-run.** If feedback files new issues or
changes a dependency, update its scheduling frontmatter through issuectl and commit the
exact rewritten issue paths before a feedback re-run consults `issuectl dag --json`.
If you capture feedback without a re-run, still record the metadata so the eventual
handoff sees the accurate live graph.

Once the feedback is absorbed (acted on via worktrees, or captured durably), the round is
done. When the session's context is filling or the user asks to wrap up, **propose
`/stint-handoff`** — the terminal wrap is a separate skill, not part of this one.

## Non-goals

- **Not a worktree**, and does not create one directly — it delegates to the
  `/worktree-*` family.
- **Does not write code** in this session — every change goes through a worktree.
- **Not for a single one-off coding task** — that's `/worktree` (router).
- **Not the terminal handoff/wrap-up** — that's `/stint-handoff` (update the `TODO.md`
  handoff narrative, verify the issuectl schedule, then `/wrap-up`).
- **Not for bare** status / deploy — those are `/worktree-status` and the project deploy
  command.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md.
