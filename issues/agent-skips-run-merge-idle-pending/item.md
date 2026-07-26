---
created: 2026-07-26
updated: 2026-07-26
type: bug
reporter: jari
status: open
priority: high
related: ['@supervisor-stuck-pending-after-self-merge', '@spinoff-must-submit-node-report']
---

_Source: orchestratectl technical-decision run (agent closing contract)_

## Description

- **Found:** 2026-07-26, by Claude Code (`/stint`) in the `frondeo-monorepo` repo, running a `technical-decision` worktree that produced an ADR.
- **orx state when observed:** binary `0.1.0`, commit `079dd628`. Run id `01kyeadrsscmx1zrt0g58tv4k9`, single node `n-0001`, kind `technical-decision`, `--headless`.
- **Severity:** High. The run is **stuck at `status: pending` forever** with the deliverable **committed but unmerged** and **no terminal report**. The supervisor stays alive polling, the tmux window never closes, and any caller trusting `run show` sees `lifecycle: pending` indefinitely — the work looks unfinished when it is actually complete-but-unlanded. A conductor had to detect and recover it by hand.

## Summary

Distinct from the already-fixed "supervisor mislabels a **merged** run" family (`@false-failed-after-merge`, `@supervisor-stuck-pending-after-self-merge`, `@orchestrated-children-hang-pending`) — in all of those the agent **did** merge and the supervisor got the status wrong. Here the agent **never merged at all**:

1. The agent did its work correctly: created the issue, wrote the ADR, and made **two clean commits on its branch** (`d2cb363` ADR + `f6b1dbb` issue-close). Worktree left clean.
2. It then **skipped its mandatory closing step** — never called `orchestratectl run merge` (nor a direct `node report`). Instead the agent session ended / dropped back to an **idle shell prompt** inside the worktree.
3. Result: branch `wt/01kyeadrss-adr-groupware-calendar` **not merged** into `main` (`git merge-base --is-ancestor` → false), `node show` → `status: pending`, `last_report: null`, `run show` → `status: pending`, supervisor `alive: true`.

The agent process was **not classified dead** — `ps -p <agent_pid>` still showed the `claude …` process alive (`S+`, ~80 min elapsed) while the tmux pane showed only the zsh prompt. So this is not the "agent-died during terminal phase" path; it is the agent **reaching a normal end-of-session without honoring the `run merge` closing contract**, and the supervisor having no fallback to detect "work committed + branch mergeable + no terminal report → the agent is done but skipped its close."

## Evidence

```
# branch has the completed work, main does not
$ git log --oneline wt/01kyeadrss-adr-groupware-calendar -2
f6b1dbb chore(issue): record ADR commit + close adr-groupware-calendar (done)
d2cb363 docs(adr): groupware/calendar architecture — Nextcloud auth + runtime
$ git merge-base --is-ancestor wt/01kyeadrss-adr-groupware-calendar main; echo $?
1   # NOT merged

# run/node both pending, no report
$ orchestratectl run show 01kyeadrsscmx1zrt0g58tv4k9   → status: pending, supervisor alive
$ orchestratectl node show … n-0001                    → status: pending, last_report: null

# agent process still alive, but pane is an idle shell
$ ps -p 12591 -o stat,etime  → S+  01:21:19
$ tmux capture-pane …        → "➜ wt-01kyeadrss-… git:(wt/…)"   (just the prompt)
```

## Recovery used (manual)

The conductor verified the two commits were genuinely complete on the branch, then finished the landing itself:

```
orchestratectl run merge 01kyeadrsscmx1zrt0g58tv4k9 --report-file <hand-authored §7.3 report>
→ merged: true, report_seq: 4; supervisor tore down worktree + branch cleanly.
```

So `run merge` works fine when invoked — the gap is purely that **the agent never invoked it and nothing else did**.

## Impact

- Silent stall of headless/autonomous runs: a `/stint` conductor blocking on `run wait` never gets a terminal status; only an out-of-band human glance ("it seems done but didn't merge") surfaces it.
- Leaks a live supervisor + tmux window + worktree per occurrence until manually recovered.
- Undermines the "settled ≠ landed, verify from git" guidance the calling skills already carry — here even *settling* never happens.

## Acceptance Criteria

- [ ] Root-cause **why the agent reaches end-of-session without calling `run merge`** on the success path (skill/closing-contract adherence vs. a wrapper that returns to shell before the close). Determine whether this is the technical-decision closing instructions specifically or affects all single-node autonomous kinds.
- [ ] Add a supervisor-side **safety net**: when the agent process ends (or goes idle past a threshold) with the branch **committed and cleanly mergeable** but **no terminal report**, either auto-run the merge+report or roll the run to a clear terminal state that names the situation — never leave it `pending` forever.
- [ ] Ensure the tmux window + worktree are not leaked when this path triggers.
- [ ] Distinguish this in `run show` from the genuinely-blocked case (needs-user) so a conductor can tell "agent skipped its close" from "agent hit a real tie."

## Tests Run

- [ ] Repro: spawn a single-node autonomous run whose agent commits but exits without `run merge`; assert supervisor terminalizes it (or auto-lands) rather than hanging `pending`.

## Implementation Notes

Reported from a live `/stint` round. Related fixes reconciled *post-merge* status (`@supervisor-stuck-pending-after-self-merge`), and mandated agents submit a terminal report (`@spinoff-must-submit-node-report`) — this issue is the **pre-merge** gap those didn't cover: the agent committed but neither merged nor reported, and stayed alive/idle rather than dying.
