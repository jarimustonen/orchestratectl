---
name: stint-handoff
description: "Terminal wrap of a work-session (työrupeama, 'stint'): surface any newly-arrived intake items, fold the human-approved ones into the next stint's agenda, then update the repo's TODO.md `## 🔄 Continue here` handoff block AND merge the execution DAG one last time, hand off via `/wrap-up`, and — if the project declares one — do the test-account reset. The light new-intake surfacing lives here (a quick listing folded into the agenda, not on-demand triage). Run at session end, on the user's go, so a fresh agent can resume from `jatketaan @TODO.md`. Use when the user says 'päätetään rupeama', 'wrap up the stint', 'hand off', 'update the handoff and wrap up', or invokes bare `/stint-handoff`. Generic across projects — reads all specifics from the repo's own AGENTS.md/TODO.md. NOT deep bug triage (that is `/triage-bugs`; this is a LIGHT surface-and-fold only); NOT the round engine (that is `/stint-start`); NOT a bare `/wrap-up` (this first updates TODO.md + the DAG, then calls it); NOT a worktree itself."
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
- **Scrutinise spin-off quality before folding.** When review-generated spin-offs (from
  `/llm-review`, a review panel, or an `/assess-findings` cascade) come up for folding into
  the next stint's agenda, weigh each one critically against real value before promoting it
  — do **not** admit them as given. Early-maturity review passes tend to over-produce
  low-value "find something to polish" suggestions, and in practice a large majority of such
  cascade spin-offs get dropped as `wontfix` rather than kept. This holds every stint: the
  human-in-the-loop fold (step 1's "Ack + fold") is exactly where a marginal suggestion
  should be dropped, not admitted.
- **Ask conversationally.** Never `AskUserQuestion` (global CLAUDE.md).
- **Read worker reports before writing the next handoff.** A terminal report is persisted as
  `last_report` on its node. Prefer the public read surface:

  ```bash
  orchestratectl run show "$run_id" --output json | jq '.data.report'
  # Node-level compatibility probe, including older binaries:
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
   before writing it. Verify a clean-ish worktree (`git status --short`) and that no round
   worker is still unsettled (any live/launched `orchestratectl run` this round has
   settled and its landing is git-verified). If workers are still running or a landing is
   unverified, **do not wrap yet** — the round isn't done; go back to `/stint-start`. Read
   the current `TODO.md` handoff block and, if the block will state deployment state, the
   project's live-version check — write "unverified" rather than guessing if you can't
   confirm it.
1. **Intake check — surface new bugs, fold approved into the next agenda.** This is the
   human-in-the-loop gate of the stint loop: nothing else invokes triage, so filed intake
   items would otherwise pile up untriaged. Do it *before* the agenda build in step 2 so
   its output feeds that build.
   - **Detect (read-only).** Query the repo's *own* issue queue for newly-arrived,
     still-untriaged intake items — the queue is the source of truth; do **not** push any
     gateway/collision logic into `issuectl`, just read it. The predicate is exactly
     `/triage-bugs`': **open** AND label **`via:telegram`** (the bot files with
     `--provenance telegram`, the primary robust signal) AND label **`needs-triage`** (the
     lifecycle label marking the still-untriaged ones). The slug shapes
     `intake-bug-<repo>-<hash>` / `tg-bug-*` are an optional sanity check **only** — never
     use them to *exclude* a correctly-labelled issue (the scheme may shift). E.g.
     `issuectl --json list --status open --label via:telegram`, then keep the ones still
     labelled `needs-triage`. (Don't key on a `status` value beyond `open` — intake
     lifecycle lives in labels, not the status enum.) A **successful** query returning
     nothing (or a project that doesn't use intake) is a fast **no-op** — say "no new
     intake" and move on; but if `issuectl` errors or the result may be incomplete, do
     **not** report "no new intake" — surface the failure instead.
   - **List — LIGHT only.** Present each new item as a single line: `- <title> — <one-line>
     (<slug>)`, where `<one-line>` is the issue's title / summary field — no generated
     analysis, no code reading. The human asks interactively for more on any item he cares
     about, or defers a deep pass to `/triage-bugs`.
   - **Ack + fold (human-gated, no silent auto-promotion).** Ask conversationally which
     items should enter the next stint. Require an **unambiguous** ack — a bare "klar"
     promotes the shown set only when it hasn't changed since you listed it; a partial or
     vague reply ("do the first two", "looks fine") must be confirmed against exact slugs
     before you treat it as approval. For **each acked** item:
     1. **Admit it out of the untriaged set** — the ack *is* the fix-now decision, so
        remove its intake hold: `issuectl label <slug> --remove needs-triage` (an item
        still labelled `needs-triage` is not "normal planned work" and would re-nag every
        handoff). This is the only reason step 3 commits rewritten issue files.
     2. **Skip duplicates** — if the slug is already a DAG node or clearly duplicates an
        issue already in the plan, don't re-fold it (close it as a duplicate or just note
        "already planned"); folding is idempotent by slug.
     3. **Fold it** into the agenda you build in step 2 — add it to the `## 🔄 Continue
        here` block and insert it into the execution DAG. To place a **lane** you may read
        the item's issue body (`issuectl show <slug>` — issue metadata, which is not the
        forbidden "code reading") to see which hot-file family it likely touches; if you
        still can't tell, **sequence it conservatively** (its own lane, or the most-likely
        lane) — never default to `UNLANED`, which asserts it touches no hot file.
     **Un-acked items:** leave them untouched at `needs-triage` (they re-surface next
     handoff — that is intended; the human can instead say "defer" →
     `issuectl label <slug> --remove needs-triage --add deferred`, or "not a bug" →
     `issuectl close <slug> --status wontfix`). If nothing is acked, fold zero and proceed
     to step 2. Nothing is promoted without the human's ack.
2. **Update the `TODO.md` handoff block** (`## 🔄 Continue here` / `ALOITA TÄSTÄ`) so a
   fresh agent can resume from `jatketaan @TODO.md` — where the round left off, what's
   landed, what prod is running, and what's next. **In the same edit, merge the execution
   DAG one last time**: drop only terminal issues, add active/non-terminal ones (including
   any intake items acked in step 1), refresh the date stamp, and set the `GLOBAL
   HEAD-OF-LINE`. This is the same merge — the active-set fetch, drop/add rules, the
   `comm -3` drift check, edge validation, and head recompute are all in the shared
   [`AGENTS-EXECUTION-DAG.md`](../stint-start/AGENTS-EXECUTION-DAG.md) § *Execution DAG
   (the convention)* — so the next resume opens onto an accurate graph.
3. **Commit the `TODO.md` handoff + DAG update immediately, on its own** — `git add
   TODO.md` (plus any issue files `issuectl` rewrote — name the exact paths, not `git add
   -A`) and commit *before* the next step, so it doesn't get folded into `/wrap-up`'s
   mixed commit or left dangling.
4. **`/wrap-up`** — it will *present proposed* `AGENTS.md` / issue / preference changes
   and ask before writing; don't assume it committed unless it reports saved changes.
5. **Test-account reset.** If the project's `AGENTS.md` / `TODO.md` declares a
   **test-account reset preference** (so testing starts from a known state), do it or
   remind the user. If the project declares none, skip this step.
6. **Verify terminal state.** Run `git status --short`. The handoff commit from step 3
   should be in; if `/wrap-up` wrote approved files without committing, follow its commit
   contract (or ask the user) — do not declare the handoff complete while main is left
   dirty. This is what lets the next agent resume from a clean tree.

## Non-goals

- **Not deep bug triage** — the intake check (step 1) is a LIGHT surface-and-fold only
  (title + one-line + slug, human-gated). Deep per-item analysis, reproduction, and code
  reading stay in `/triage-bugs`; the human can ask for more on any item interactively.
- **Not the round engine** — it does not pull, plan, spawn worktrees, or deploy; that is
  `/stint-start`.
- **Not a bare `/wrap-up`** — this first updates the `TODO.md` handoff block and merges
  the DAG, *then* calls `/wrap-up`.
- **Not a worktree**, and does not create one.
- **Does not write product code** — the only direct edits are the `TODO.md` handoff block
  and the DAG merge (plus any issue files `issuectl` necessarily rewrote while merging);
  `/wrap-up` may separately propose other changes.
- **Hardcodes no project facts** — reads them from the repo's AGENTS.md/TODO.md.
