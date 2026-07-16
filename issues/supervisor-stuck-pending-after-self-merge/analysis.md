Spinoff supervisor stays alive at `status: pending` and never runs teardown even though the agent self-merged its branch successfully. The run's `events.jsonl` stops at `supervisor.started` — no `node.report`, no merge event, no status transition is ever recorded — so the supervisor polls an empty inbox forever, the tmux window + supervisor process leak, and `run show`/`run wait`/`run list` report a false `pending`.

## Observed (deutschpad, orchestratectl 0.1.0, commit 64b077d)

A batch of **9 `--kind spinoff --headless` runs** was spawned off `main`. **All 9 self-merged successfully** (verified from git: each branch's commits + merge commit are in `main`; `git branch -d` on each succeeded = merged). But only **4 tore down cleanly**; **5 got stuck**:

- Clean (status=done, tmux window gone, supervisor exited): d8-enable, a11y, irregverbs, universal-ask-ai
- **Stuck (status=pending, tmux window alive, supervisor alive ~21.7h):**
  `01kxj8gzvs…` p24-design, `01kxj8h4rj…` grammar-card, `01kxj8hksy…` nuance-vn, `01kxj92stj…` vocab-backfill, `01kxj98drh…` irr-comparative

Same batch, same spawn command, same merge path → **intermittent race** in the merge→report→teardown handoff.

## Evidence (identical across all 5 stuck runs)

`~/.orchestratectl/runs/01kxj8gzvs…/events.jsonl` — only 3 events, last is `supervisor.started`:
```
seq 1 run.created
seq 2 node.created  (agent_pid 63757, branch wt/01kxj8gzvs-p24-design, tmux @11)
seq 3 supervisor.started  (pid 63840)
```
No `node.report`, no merge/teardown event, ever.

`nodes/n-0001.json`: `"status": "pending"`, `"last_report": null`.
`manifest.json`: `"status": "pending"`, `updated_at` = 5s after creation (never advanced).
`supervisor.state.json`: `"last_seq_own": 3` (processed nothing after its own start), `updated_at` recent (still polling).
Process: `ps -p 63840` → alive, ELAPSED 21:41:26, `orchestratectl supervise 01kxj8gzvs…`.

**Git ground truth (contradicts the run status):** merge commit `af0ebcb "merge: P2.4 auto-gen-mastery-gating reconciliation design (spinoff)"` is in `main`; the work landed. The agent completed and merged; the supervisor just never learned about it.

## Root-cause hypothesis

The agent's terminal step (self-merge via `run merge` / terminal `node report`) **committed the git merge but never appended the `node.report` event to the run's `events.jsonl`** (or appended it to a place the supervisor doesn't consume — `last_seq_own` stays at 3). With no terminal report in the event log:
- the supervisor never marks node/run `done`,
- teardown (tmux `kill-window` + worktree/branch removal) never fires,
- the supervisor loops forever (no terminal condition), leaking a process + tmux window per stuck run.

Because it's 5/9 in one batch, the write of the terminal report (or the supervisor's observation of it) appears **racy under concurrency** — likely a lost-update / ordering issue between the merging agent writing the report and the supervisor's poll/seq bookkeeping, or the merge path exiting before the report event is durably flushed.

## Impact

- **Callers cannot trust run status.** `run show`/`run wait`/`run list` report `pending`/`failed` for work that actually merged — the documented "branch on status to detect completion" contract is unsafe. (Consumers must git-verify every landing, which the deutschpad `/stint` skill already works around.)
- **Resource leak:** one live supervisor process + one tmux window per stuck run, indefinitely (21+h observed).
- **Manual teardown required:** `git worktree remove` + `git branch -d` + `tmux kill-window` + kill the supervisor pid, per stuck run.

Related to the previously-noted (but not filed here) "false failed despite successful merge" observation — same family: **run status diverges from git reality after a successful self-merge.**

## Repro sketch

Spawn a high-fan-out batch (≈9) of `--kind spinoff --headless` runs off the same branch that each do real work and self-merge. Expect a fraction to end at `status: pending` with a live supervisor and a surviving tmux window, `events.jsonl` truncated at `supervisor.started`, despite the merge commit being present in the target branch.

## Suggested direction

1. Make the terminal `node.report` append **atomic + durably flushed before the merging process exits**, and have the supervisor treat "branch merged into target" (git-observable) as a fallback terminal signal so a lost report event can't strand it forever.
2. Give the supervisor a **teardown-on-detected-merge** path + a watchdog/timeout so it cannot poll an empty inbox indefinitely.
3. Emit a `node.report`/merge event on the `run merge` code path unconditionally (even when invoked by the agent), and assert `last_seq_own` advances past it.
