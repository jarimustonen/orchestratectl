---
name: stint-handoff
description: "Terminal wrap of a work-session (työrupeama, 'stint'): update the repo's TODO.md `## 🔄 Continue here` handoff narrative, verify the live issuectl scheduling DAG, then hand off via `/wrap-up` and, if the project declares one, do the test-account reset. Run at session end, on the user's go, so a fresh agent can resume from `jatketaan @TODO.md`. Use when the user says 'päätetään rupeama', 'wrap up the stint', 'hand off', 'update the handoff and wrap up', or invokes bare `/stint-handoff`. Generic across projects: reads all specifics from the repo's own AGENTS.md/TODO.md and issue metadata. NOT the round engine (that is `/stint-start`, which spawns worktrees and deploys); NOT a bare `/wrap-up` (this first updates TODO.md and verifies scheduling, then calls it); NOT a worktree itself."
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Stint-handoff — the terminal wrap

You are the **orchestrator** closing out a stint (työrupeama). This skill is the
**terminal wrap only**: leave the repo in a state a fresh agent can resume from. It does
**not** spawn worktrees, deploy, or run a round — that is the round engine
**`/stint-start`**. Run this at session end, on the user's go.

A stint typically fills a session's context after ~one round. When you notice that (or
the user asks), **propose** the handoff; run the steps below only on the user's go — do
not auto-run them.

This skill is **generic**: project specifics (the test-account reset preference and where
the `TODO.md` handoff block lives) are read from the repo's own `AGENTS.md` / `TODO.md`.
Scheduling comes only from `issuectl dag --json`; `TODO.md` remains a narrative and never
stores a second scheduling graph. Verify `issuectl dag --help` advertises
`--reservations`. If the command or required JSON surface is unavailable, stop and report
an unmigrated or incompatible project rather than falling back to prose.

## Standing discipline

- **Keep main clean.** The handoff edits are orchestration, not product code — you make
  them in this session. But commit them promptly and on their own (see the commit step); never
  leave `TODO.md` modified-but-uncommitted across the wrap.
- **Read scheduling, never duplicate it.** Use `issuectl dag --json` for lane order,
  dependency state, collision tokens, computed heads, and spawnability. Do not copy those
  fields into the handoff narrative.
- **Scrutinise spin-off quality before folding.** Before filing or adding a
  review-generated spin-off (from `/llm-review`, a review panel, or an
  `/assess-findings` cascade) to the next stint's agenda, weigh it critically against real
  value. Early-maturity review passes tend to over-produce low-value "find something to
  polish" suggestions, and in practice a large majority of such cascade spin-offs get
  dropped rather than kept.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).
- **Read worker reports before writing the next handoff.** A terminal report is persisted as
  `last_report` on its node. For a single-worker run, prefer the public
  `run show` surface. For a multi-node run, inspect each node:

  ```bash
  # skill-example-ci: skip (the parser validates CLI argv, not shell pipelines)
  orchestratectl run show "$run_id" --output json | jq '.data.report'
  # Node-level projection-compatible probe:
  # skill-example-ci: skip (the parser validates CLI argv, not shell pipelines)
  orchestratectl node show "$run_id" n-0001 --output json |
    jq '.data.report // .data.last_report'
  ```

  `run wait` is multi-run: its results are in `data.runs[]`, so probe it with
  `jq '.data.runs[] | {run_id, status, summary}'`, not `.data.status`. The wait
  result folds in a summary; `run show`/`node show` expose the complete
  `discussion_items`, `spinoff_proposals`, and `wrap_up_recommendations` needed
  for the next handoff.
- **Propose, don't presume.** `/wrap-up` presents proposed `AGENTS.md`/issue/preference
  changes and asks before writing; don't assume it committed unless it reports saved
  changes.

## Steps (propose; run only on the user's go)

0. **Preflight (read-only).** This is the terminal wrap, not a re-orient — but because it
   can be invoked standalone, confirm the ground truth you're about to record is real
   before writing it. Verify a clean-ish worktree (`git status --short`). Inspect
   `orchestratectl run list --output json` and relevant `run show` records, not just runs
   remembered in this conversation. Every live, awaiting-input, recoverable, or otherwise
   resumable worker must have landed or relinquished ownership through a terminal
   cancel/abandon path that confirms no preserved worktree, branch, or resumable work
   remains. If ownership stays unresolved, skip schedule verification, record the run id,
   slug, and preserved-work fact in the narrative, commit that narrative, and stop before
   `/wrap-up`; never end the session with no durable record. Otherwise read the current
   `TODO.md` handoff block and, if the block will state deployment state, the
   project's live-version check — write "unverified" rather than guessing if you can't
   confirm it.
1. **Read the live schedule.** After preflight proves there are no ownership holds, run
   `issuectl dag --json --reservations '[]'` and read `.data.lanes[]`,
   `.data.unscheduled`, and `.data.spawnable_heads`. If the command fails, its JSON is
   malformed, or the graph has a missing blocker, self-dependency, or cycle, record the
   verification failure for the narrative and continue only through the narrative commit;
   do not call `/wrap-up` or declare the handoff complete. Do not mutate issue scheduling
   during terminal wrap or encode a workaround in `TODO.md`.
2. **Update only the `TODO.md` handoff narrative** (`## 🔄 Continue here` / `ALOITA
   TÄSTÄ`) so a fresh agent can resume from `jatketaan @TODO.md`: where the round left
   off, what landed, what is live, the intended product direction, and unresolved
   decisions. Issue slugs may provide context, including a concise "needs scheduling
   triage" note for unscheduled active work or a run-ownership fact from preflight.
   Recording that verification failed and naming involved slugs is an unresolved decision,
   not a copied schedule. Do not describe any issue as currently ready,
   blocked, headed, or spawnable, and do not copy lane order, dependency edges, collision
   values, computed-head flags, or spawnability into this file.
3. **Commit the handoff update immediately, on its own**: `git add TODO.md` (that exact
   path, not `git add -A`) and commit before the next step, so it is not folded into
   `/wrap-up`'s mixed commit or left dangling. If the narrative did not change, do not
   manufacture an empty commit.
4. **`/wrap-up`**: if schedule verification failed in step 1, stop here and report the
   committed narrative plus the failure. Otherwise `/wrap-up` will present proposed
   `AGENTS.md` / issue / preference changes and ask before writing; do not assume it
   committed unless it reports saved changes.
5. **Test-account reset.** If the project's `AGENTS.md` / `TODO.md` declares a reset
   preference, do it or remind the user. If the project declares none, skip this step.
6. **Verify terminal state.** Run `git status --short`. If `/wrap-up` wrote approved files
   without committing, follow its commit contract (or ask the user). Do not declare the
   handoff complete while main is dirty.

## Non-goals

- **Not the round engine** — it does not pull, plan, spawn worktrees, or deploy; that is
  `/stint-start`.
- **Not a bare `/wrap-up`** — this first updates the `TODO.md` handoff narrative and
  verifies the issuectl schedule, then calls `/wrap-up`.
- **Not a worktree**, and does not create one.
- **Does not write product code** — its only direct edit is the `TODO.md` handoff
  narrative; `/wrap-up` may separately propose other changes.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md and issue
  metadata.
