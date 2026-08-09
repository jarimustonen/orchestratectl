---
created: 2026-07-13
updated: 2026-08-09
type: bug
reporter: claude-code
status: fixed
priority: high
commits:
- hash: ec20128
  summary: reconcile run status with git after self-merge
- hash: 275d152
  summary: harden reconcile against live-work loss (llm-review)
related: ['@supervisor-stuck-pending-after-self-merge']
closed: 2026-07-16
---

# spinoff reported failed/agent-died despite branch already merged to target (false negative run status)

_Source: orchestratectl supervise (liveness poll → agent-died classification)_

## Description

- **Found:** 2026-07-09, by Claude Code while running a multi-worktree rupeama in the `deutschpad` repo.
- **orx state when observed:** `main` @ `87a5bd1`, binary rebuilt from source via `cargo install --path crates/octl-cli` (v0.1.0).
- **Severity:** High. The run status is a **false negative**: it says the work failed and the branch is unmerged when in fact the agent completed, committed, and fast-forward-merged into the target branch. This silently inverts the truth for anyone (human or agent) who trusts `run show`.

_(Filed from the root `BUG-false-failed-despite-successful-merge.md` triage file, 2026-07-13.)_

## Summary

When a spinoff agent finishes its work — commits and **fast-forward-merges its branch into the target** — and then its agent process exits/stops responding during the *terminal report/cleanup phase*, the supervisor:

1. Records a node report with `failed: true`, `reason: "agent-died"`, `summary: "Agent for node n-0001 stopped responding: agent-died"`.
2. Rolls the run up to `status: failed`.
3. Runs a "blocked report" cleanup that logs **`branch <b> left unmerged for you to merge (blocked report; worktree preserved at …)`** and emits a `cleanup.branch_preserved` event.

**All of this is wrong when the branch already merged.** The supervisor never checks whether the branch tip is already an ancestor of / reachable from the target branch before declaring it unmerged and the run failed.

Observed **3 for 3** in one session: every spinoff whose agent died right at the end was mislabeled `failed`, yet every one had already landed on `main`.

## Evidence

Three independent spinoff runs, each reported `failed` with identical `agent-died` node reports and identical "branch left unmerged" cleanup lines:

| run_id | title | manifest.status | node.report.reason | git reality |
|---|---|---|---|---|
| `01kx2yyeff9s9z65r26k76s72v` | EP C45 prefix-family + separability | `failed` | `agent-died` | **merged** → `main` ff to `0465848` |
| `01kx31zkkan755j0e9xc72n33f` | EP C6 noun-plurals | `failed` | `agent-died` | **merged** → `main` ff to `2ae0aac` |
| `01kx34r35jj01q3y35rjj7qtfx` | SYN C3 recognition-sense | `failed` | `agent-died` | **merged** → `main` ff to `121d6a2` |

### Target-repo reflog (ground truth — the merges happened)

```
121d6a2 refs/heads/main@{1}: merge wt/01kx34r35j-syn-c3-recognition-sense-disambiguation: Fast-forward
2ae0aac refs/heads/main@{2}: merge wt/01kx31zkka-ep-c6-noun-plurals: Fast-forward
0465848 refs/heads/main@{3}: merge wt/01kx2yyeff-ep-c45-prefix-family-separability: Fast-forward
```

Each merged branch also carried a self-authored `chore(issue): close …(done)` commit as its tip — i.e. the agent had reached its *final* step (issue close) before the process ended. The death happened at or after merge, not during the work.

### The contradicting supervisor output (example: EP C45 `01kx2yyeff…`)

`events.jsonl`:
```json
{"seq":4,"kind":"node.report","node_id":"n-0001","data":{"failed":true,"reason":"agent-died",
  "summary":"Agent for node n-0001 stopped responding: agent-died", ...}}
{"seq":5,"kind":"run.status","data":{"status":"failed"}}
{"seq":6,"kind":"cleanup.window_missing", ...}
{"seq":7,"kind":"cleanup.branch_preserved","data":{"branch":"wt/01kx2yyeff-…","reason":"blocked report",
  "worktree_path":"…/wt-01kx2yyeff-…"}}
{"seq":8,"kind":"cleanup.session_killed","data":{"session":"headless"}}
{"seq":9,"kind":"supervisor.exited","data":{"reason":"work-complete","iterations":3817}}
```

`supervisor.stderr.log`:
```
supervisor cleanup: branch wt/01kx2yyeff-ep-c45-prefix-family-separability left unmerged for you to
  merge (blocked report; worktree preserved at …/wt-01kx2yyeff-…)
```

The branch was **already merged into `main`** at that moment. The "left unmerged … worktree preserved" claim is false.

### Extra inconsistency: "worktree preserved" but worktree + branch are gone

The cleanup claims `worktree preserved at <path>`, but afterwards **the worktree directory, the `.git/worktrees/<name>` admin dir, and the branch ref were all absent** (`git worktree list` empty, `git branch` has no `wt/…`, path does not exist). So the "preserved for you to merge" contract was not honored either — nothing was left to merge, and had the merge *not* already happened, the work would have been lost. This makes the false report doubly dangerous: it tells you to go rescue a worktree that isn't there.

## Impact

1. **Trust destroyed** — run status cannot be believed. In this session the reporter had to `git log`/reflog the target repo after *every* spinoff to discover the work had actually landed. The consuming project's `CLAUDE.md` already carries a standing warning ("run-status on epäluotettava → varmista aina main suoraan") — this bug is the upstream cause.
2. **Duplicate-work risk** — a false `failed` invites re-running an already-merged unit. The reporter nearly relaunched EP-C45 on top of an already-merged one; only an out-of-band catch prevented a duplicate.
3. **False data-loss scare / actual data-loss risk** — "branch left unmerged, worktree preserved" points a rescuer at a nonexistent worktree. If the merge had *not* completed, the same cleanup path would strand or drop the work while claiming it was preserved.

## Root-cause hypothesis

The supervisor's liveness poll detects the agent process is gone and takes the `agent-died` → `failed` → "blocked report" branch **unconditionally**, without reconciling against git. Two things seem to be missing:

1. **A merge-state check before declaring failure/unmerged.** Before emitting `failed` + `branch_preserved`, the supervisor should ask git whether the branch already merged into the target — e.g. `git merge-base --is-ancestor <branch-tip> <target>` (or `git branch --merged <target>`), or compare the worktree HEAD against the target's history. If already merged → report **success** (or at minimum `merged`, terminal-ok), and clean up the branch/worktree normally instead of "preserving" it.
2. **A success-report race.** The agent's final actions are (commit → merge → `node report` success → exit). It looks like the process can exit before/while the terminal success report is consumed, so the poll wins the race and stamps `agent-died`. The supervisor should treat "process gone" as *inconclusive*, then reconcile: read any pending report **and** check git merge state before choosing failed vs. success. "Process no longer alive" ≠ "work failed."

## Reproduction (sketch)

1. `orchestratectl run create --kind spinoff --headless …` with a task the agent can finish.
2. Let the agent complete: commit, fast-forward merge into the source branch, close its issue.
3. Have the agent process exit at/after the merge, slightly before/while the supervisor consumes the terminal success report (in the wild this happened naturally — a ~50-min run ending in the merge, `iterations: 3817`).
4. Observe: `run show` → `status: failed`, node report `agent-died`, stderr "branch left unmerged", **while `git log <source>` shows the merge already landed.**

## Suggested fix (priority order)

1. **Reconcile with git before terminal classification.** On `agent-died`/liveness-loss, do NOT immediately fail. First: (a) read any final node report on disk; (b) run `git merge-base --is-ancestor <branch> <target>`. If merged → terminal **success**; branch/worktree teardown proceeds normally. Only if *not* merged and no success report → `failed` + genuinely preserve the worktree.
2. **Make "preserved" real.** If the cleanup path claims `worktree preserved`, it must actually leave the worktree + branch intact (and it must not run when the branch already merged). Today it prints "preserved" and the worktree is gone — pick one and make the event truthful.
3. **Distinguish "process exited cleanly after work" from "agent crashed."** `supervisor.exited reason=work-complete` co-occurring with a `failed` rollup is self-contradictory; the exit reason suggests the supervisor itself knew the work was complete.

## Secondary observations (resilience — related but distinct from the bug above)

Same session, caused by a **server power outage + reboot** that killed all supervisors/tmux mid-run. Not the false-failed bug, but worth logging (may warrant their own issues):

- **Stranded uncommitted work.** One run (`01kwxmmy1r`, EP-C3 multi-phase-core) had its supervisor killed by the reboot while the agent had **substantial uncommitted work in the worktree, no commits**. Nothing auto-resumed; recovered by hand (commit → rebase → verify → merge). A crash-recovery/resume path, or periodic auto-commit checkpoints in the worktree, would prevent loss.
- **`pending` forever.** A launched run whose supervisor died (`sup:none`) stayed `status: pending` indefinitely with no reaper to mark it `failed`/`cancelled`. A stale-supervisor sweep (or a heartbeat TTL) would close these out. (`01kx2rv2d0…`, a fully empty EP-C45 attempt: pending, sup:none, no commits, empty tmux pane — required manual `run cancel` + `git worktree remove` + `branch -D`.)
- **State dir wiped by reboot.** `~/.orchestratectl/runs/*` for older runs were gone after the reboot (tmpfs-like loss for some paths; `/private/tmp` scratchpads also cleared). If any run state is expected to survive a reboot, it should live outside volatile storage.

## Workaround (until fixed)

**Never trust `run show` status for spinoffs. After any terminal (or even non-terminal) status, verify the target branch directly:** `git log --oneline <target>`, `git reflog | grep "merge wt/<id>"`, and `git worktree list`. Treat `failed`/`agent-died` as "go check git," not as "work lost."

## Comments

### 2026-08-09T04:02:09Z · @claude-intakectl-stint

Observed again on orchestratectl binary **0.1.0** during an intakectl stint (2026-08-08), across multiple runs. `run wait`/`run show` reported `status: failed` and/or `landed: false` (`landed_method: unverified` or `report-marker`) for runs whose work was in fact git-verified on main (worker-agent-spawn 01kzbj5bwn... landed via report-marker; the extract technical-decision + spinoffs similarly). No false negative caused data loss because I always git-verify landing independently, but the status is misleading. Same version caveat as interactive-code-run-self-merged: this is likely a stale-0.1.0-binary repro (skills already ship for 0.1.1/0.1.3), not a regression.
