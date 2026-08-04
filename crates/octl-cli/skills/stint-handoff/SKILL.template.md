---
name: stint-handoff
description: "Terminal wrap of a work-session (työrupeama, 'stint'): update the repo's TODO.md `## 🔄 Continue here` handoff block AND merge the execution DAG one last time (committed on its own), then hand off via `/wrap-up`, and — if the project declares one — do the test-account reset. Run at session end, on the user's go, so a fresh agent can resume from `jatketaan @TODO.md`. Use when the user says 'päätetään rupeama', 'wrap up the stint', 'hand off', 'update the handoff and wrap up', or invokes bare `/stint-handoff`. Generic across projects — reads all specifics from the repo's own AGENTS.md/TODO.md. NOT the round engine (that is `/stint-start`, which spawns worktrees and deploys); NOT a bare `/wrap-up` (this first updates TODO.md + the DAG, then calls it); NOT a worktree itself."
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

This skill is **generic**: project specifics (the test-account reset preference, where
the `TODO.md` handoff block lives) are read from the repo's own `AGENTS.md` / `TODO.md`.
The Execution-DAG convention and the final-merge procedure live in the shared reference
**[`AGENTS-EXECUTION-DAG.md`](../stint-start/AGENTS-EXECUTION-DAG.md)** (installed
alongside `stint-start`); this skill LINKS there rather than repeating the rules.
**Open and read that file before the DAG merge in step 1** — Claude Code loads only this
`SKILL.md`, so the merge algorithm is not in context until you open the link. If it is
missing or unreadable, stop and report an incomplete skill install rather than
improvising the merge from memory.

## Standing discipline

- **Keep main clean.** The handoff edits are orchestration, not product code — you make
  them in this session. But commit them promptly and on their own (see step 2); never
  leave `TODO.md` modified-but-uncommitted across the wrap.
- **Never regenerate the DAG — merge it.** The final DAG update is a stateful *merge*
  (drop only terminal issues, add active/non-terminal ones, keep the existing lane order),
  exactly the Phase-0 merge `stint-start` runs at the start of a round. Regenerating from
  scratch risks dropping a `collision:` edge.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).
- **Propose, don't presume.** `/wrap-up` presents proposed `AGENTS.md`/issue/preference
  changes and asks before writing; don't assume it committed unless it reports saved
  changes.

## Steps (propose; run only on the user's go)

0. **Preflight (read-only).** This is the terminal wrap, not a re-orient — but because it
   can be invoked standalone, confirm the ground truth you're about to record is real
   before writing it. Verify a clean-ish worktree (`git status --short`) and that no round
   worker is still unsettled (any live/launched `orchestratectl run` this round has
   settled and its landing is git-verified). If workers are still running or a landing is
   unverified, **do not wrap yet** — the round isn't done; go back to `/stint-start`. Read
   the current `TODO.md` handoff block and, if the block will state deployment state, the
   project's live-version check — write "unverified" rather than guessing if you can't
   confirm it.
1. **Update the `TODO.md` handoff block** (`## 🔄 Continue here` / `ALOITA TÄSTÄ`) so a
   fresh agent can resume from `jatketaan @TODO.md` — where the round left off, what's
   landed, what prod is running, and what's next. **In the same edit, merge the execution
   DAG one last time**: drop only terminal issues, add active/non-terminal ones, refresh
   the date stamp, and set the `GLOBAL HEAD-OF-LINE`. This is the same merge — the active-set
   fetch, drop/add rules, the `comm -3` drift check, edge validation, and head recompute
   are all in the shared
   [`AGENTS-EXECUTION-DAG.md`](../stint-start/AGENTS-EXECUTION-DAG.md) § *Execution DAG
   (the convention)* — so the next resume opens onto an accurate graph.
2. **Commit the `TODO.md` handoff + DAG update immediately, on its own** — `git add
   TODO.md` (plus any issue files `issuectl` rewrote — name the exact paths, not `git add
   -A`) and commit *before* the next step, so it doesn't get folded into `/wrap-up`'s
   mixed commit or left dangling.
3. **`/wrap-up`** — it will *present proposed* `AGENTS.md` / issue / preference changes
   and ask before writing; don't assume it committed unless it reports saved changes.
4. **Test-account reset.** If the project's `AGENTS.md` / `TODO.md` declares a
   **test-account reset preference** (so testing starts from a known state), do it or
   remind the user. If the project declares none, skip this step.
5. **Verify terminal state.** Run `git status --short`. The handoff commit from step 2
   should be in; if `/wrap-up` wrote approved files without committing, follow its commit
   contract (or ask the user) — do not declare the handoff complete while main is left
   dirty. This is what lets the next agent resume from a clean tree.

## Non-goals

- **Not the round engine** — it does not pull, plan, spawn worktrees, or deploy; that is
  `/stint-start`.
- **Not a bare `/wrap-up`** — this first updates the `TODO.md` handoff block and merges
  the DAG, *then* calls `/wrap-up`.
- **Not a worktree**, and does not create one.
- **Does not write product code** — the only direct edits are the `TODO.md` handoff block
  and the DAG merge (plus any issue files `issuectl` necessarily rewrote while merging);
  `/wrap-up` may separately propose other changes.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md.
