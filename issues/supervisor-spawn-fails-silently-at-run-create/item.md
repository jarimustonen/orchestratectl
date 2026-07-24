---
created: 2026-07-22
updated: 2026-07-24
type: bug
status: open
priority: high
---

# run create: supervisor spawn fails silently (agent never launches, status stuck pending, no stderr log)

## Description

During a real `/stint` session (2026-07-22 iltapäivä), spawning autonomous
`--kind spinoff` runs began failing **silently at creation**:

- `orchestratectl run create --kind spinoff …` **hangs and hits the caller's
  2-minute timeout** (no JSON envelope returned to stdout).
- The run **is** created (`run.created` event fires, worktree materialises,
  manifest exists, appears in `run list`).
- But the **supervisor never actually starts**: manifest shows
  `status: pending`, `supervisor.alive: false`, `updated_at` frozen at
  `run.created` time.
- **No `supervisor.stderr.log` file is written** in the run dir — only
  `events.jsonl` (single `run.created` line), `manifest.json`,
  `supervisor.state.json`. So there is no error trace to diagnose from.
- **No agent tmux window** is ever created in the `headless` session.

Reproduced **3 consecutive times** (v1 `01ky56ms7j`, v2 `01ky5757w4`,
v3 `01ky58bv1a`) with identical prompt-file + `--source-branch main`, each with
a fresh `--idempotency-key`. Every attempt: hang → timeout → orphan `pending`
run with dead supervisor and no agent.

## Not a total outage

Other spinoff runs spawned **earlier in the same session** (by this and a
parallel session) came up fine and their agent windows are alive in `headless`
(e.g. `wt-01ky568arf-t0-codeharness-adapter`,
`wt-01ky568gn1-t2-plan-json-v2-schema`). So the mechanism works in general — this
looks like an intermittent/stateful failure in the **supervisor fork/exec at
`run create`** that, once it starts happening, reproduces reliably for new runs.

## `run reattach` does NOT fully recover

For v2, `run reattach 01ky5757w4…` returned `action: reattached`,
`supervisor_pid: 24947`, and subsequently `supervisor.alive: true` with events
`supervisor.reattach-requested / started / reattached`. **But the agent still
never launched** — no agent window in `headless`, manifest stayed `pending`, zero
commits, worktree untouched at its base commit for >1h. So reattach revives the
*supervisor process* but not the *agent spawn* it was supposed to perform.

## Repro (observed)

```
orchestratectl run create --kind spinoff \
  --title "groups-first-access-v3" \
  --prompt-file <path> --source-branch main \
  --idempotency-key groups-first-access-v3-20260722
# → hangs ~100s, no stdout JSON
orchestratectl run show <id> --output json
# → manifest.status = pending, supervisor.alive = false, updated_at == created_at
ls ~/.orchestratectl/runs/<id>/
# → NO supervisor.stderr.log; events.jsonl has only run.created
tmux list-windows -t headless   # → no window for this run
```

## Impact

Blocks autonomous orchestration mid-stint: the conductor cannot spawn new
spinoff units. Worse, the failure is **silent** — `run create` hangs rather than
returning a `supervisor_spawn_failed` error envelope, and no stderr log is
written, so there is nothing to act on without deep manual inspection. A caller
that trusts the (never-arriving) envelope would stall.

## Suggested fixes

1. **Fail loudly, not silently.** If the supervisor does not confirm start within
   a bounded time, `run create` should return the documented
   `supervisor_spawn_failed` error envelope (with the run id, so the caller can
   inspect/teardown) instead of hanging until the caller's timeout.
2. **Always write `supervisor.stderr.log`** (even an empty/early one) at spawn, and
   capture the fork/exec failure reason into it — right now there is zero trace.
3. **`run reattach` should (re)perform the agent spawn**, not just revive the
   supervisor process — otherwise a reattached run is a zombie (supervisor up,
   agent absent, status pending forever).
4. Investigate the **stateful trigger**: what accumulates during a session (fd
   exhaustion? tmux server limit? a lock left by a torn-down run? worktree-root
   contention?) that flips creation from working → reliably failing.

## Environment

- orchestratectl 0.1.0 (commit a54f0ff6)
- macOS (Darwin 25.5.0), tmux sessions `default`/`headless`/`codetest` all healthy
- Repo: 3dbear-monorepo, heavy parallel-session day (main moved ~19 commits under
  the stint via other sessions; many worktrees created/torn down in the session).

## Corroboration + new evidence (2026-07-23, re-diagnosed from a session whose worktree was deleted under it)

A follow-up session (the k.3 `first_access` task, spawned into
`wt-01ky58bv1a-groups-first-access-v3`) had **its worktree and branch torn down
mid-run**; the run `01ky58bv1a…` is now `cancelled`, `node_count:0`,
`worktree_root:null`. Re-inspecting the three runs confirms this issue exactly
and adds a mechanism detail for suggested-fix #3:

**v2 supervisor false-completed after spawning nothing.** From
`~/.orchestratectl/runs/01ky5757w4…/`:

- `supervisor.stderr.log`:
  `{"reason":"work-complete","iterations":1188,...}` — the reattached supervisor
  ran **1188 poll iterations over ~18 min** and then exited claiming
  **`work-complete`**.
- `supervisor.state.json`: `"spawned_children": {}` — it completed with **zero
  children ever spawned**.
- Global `orchestratectl.log.jsonl` for the v2 window shows only
  `Reattach → Supervise → supervisor started (pid 24947)` then an ~18-min gap of
  nothing, then `Cancel`. **No `CreateNode`, no spawn dispatch, no `create.sh`
  invocation is ever logged.**

So the reattached supervisor doesn't just "fail to spawn the agent" (fix #3) — it
treats a run with an unspawned child as **already complete** and exits
successfully. That is a distinct false-positive terminal state worth a guard:

5. **A supervisor for an autonomous single-node run must not report
   `work-complete` while `spawned_children` is empty and `node_count == 0`.**
   Reaching the poll/idle terminal condition with no child ever spawned should be
   a `supervisor_spawn_failed` (or `no-child-spawned`) terminal, not
   `work-complete` — otherwise the run silently looks "done" while nothing ran.

**v1 / v3 never started a supervisor at all.** Confirmed the two directories
contain only `events.jsonl` + `manifest.json` (no `supervisor.stderr.log`,
no `supervisor.state.json`), matching the "silent spawn failure, zero trace"
signature above. Only v2 (the reattached one) has supervisor artifacts.

**Disposition:** worktree deletion mid-session forced a fresh session; the actual
first_access work will proceed via interactive `/worktree-code` (not another
autonomous spinoff) to route around this bug. This issue stays `open` as the
root-cause tracker.

## New evidence (2026-07-24) — reproduces under a `/fan-out` load; adds a second failure shape (supervisor started then died)

Another `/stint` session (`s2-canvas-lti-passback`, the S2 Canvas-LTI grade
passback work) hit this again while fanning out **8 `--kind fan-out` children +
2 spinoffs** in a tight window (~07:00–07:40). Of ~11 runs spawned, **9 landed
cleanly** (agent ran, committed, self-merged, verified in git) and **4 got stuck
in `pending` with a dead/absent supervisor**. Two distinct signatures appeared:

**Signature A — supervisor never started (identical to the original report).**
- `s2-canvas-lti-v2/puhekielen-tulkkinauha` (`01ky9fgmfx005h9ytjd4j913h2`):
  `events.jsonl` has **only `run.created`** (1 line) — no `node.created`, no
  `supervisor.started`. `manifest.supervisor.pid = None`, `alive = None`,
  `status = pending`, `updated_at == created_at` (07:12:55, frozen). No
  `supervisor.stderr.log`. Worktree materialised but sat at base commit
  (`29ca1ec93`) with zero commits — agent never launched. Exactly the "silent
  spawn failure, zero trace" signature.

**Signature B — supervisor DID start, then died, leaving `pending` (new).**
- `canvas-lti-v3-bcf` (`01ky9fms4gyqde21ac5te08fqt`): `events.jsonl` has
  `run.created` → `node.created` → `supervisor.started` (3 lines) — so the
  supervisor **did** come up. But then silence: agent never committed (worktree
  at base commit, 0 commits), supervisor later shows `pid=None, alive=None`, and
  the run is frozen at `status: pending` (`updated_at` 07:16:23). This is *past*
  the spawn point of Signature A but still terminates in the same stuck-`pending`
  state — so the guard in suggested-fix #5 ("no `work-complete` with empty
  children") must also cover *supervisor death after start* transitioning the run
  to a terminal `failed`, not leaving it `pending` indefinitely.

**Two of the four "stuck" runs had actually completed their work** — the dead
supervisor just never terminalised them:
- `s2-canvas-lti-v2/yhdyssanapommi` (`01ky9gjvcxbb…`): agent committed **and
  self-merged** — commit `9ef00d7a3` is in `main`, worktree already torn down —
  yet `run wait` / `run show` still needed a git cross-check to confirm (the run
  did eventually read `done`, but only after the merge; during the stall window
  it read `pending`). Matches the existing
  `supervisor-stuck-pending-after-self-merge` issue.
- `s2-canvas-lti-v2/kayttolupa-detektiivi` (`01ky9g40db…`): agent committed its
  full work (`99f696a6d`, game + Playwright spec, +228 lines) but **had not
  merged** — worktree left with an uncommitted `test-results/` (Playwright run
  artifact) which then made `run merge` fail with `merge_failed`
  (`git rebase main` → "cannot rebase: You have unstaged changes"). After manually
  removing the untracked `test-results/` dir, `run merge` succeeded and it landed
  as `bf34326c8`.

### Two additional problems this session surfaced

6. **`run merge` is blocked by agent-left untracked build artifacts.** The
   Playwright test run wrote `test-results/` into the worktree; the agent
   committed its source but not that dir (correctly — it's an artifact), yet
   `run merge`'s `git rebase main` refuses with "You have unstaged changes" and
   returns `merge_failed`. The merge step should either (a) ignore untracked
   artifacts when deciding whether the tree is clean enough to rebase, or (b)
   surface a clearer error than a raw rebase failure. Consider shipping a default
   `.gitignore` entry for `test-results/` in worktree scaffolding, or having the
   merge step stash/clean untracked files before the rebase.

7. **`run wait` is effectively unusable for a batch here — argument parsing.**
   Passing several run-ids to `orchestratectl run wait` as documented
   ("pass several run-ids to block until all settle") repeatedly failed with
   `invalid_run_id` when the ids arrived as a single space- or newline-separated
   argument (shell expansion / word-splitting in a non-interactive `bash -c`
   context). Every multi-id `run wait` in this session returned
   `invalid_run_id` **immediately** (exit non-zero) instead of waiting, so the
   conductor fell back to polling `manifest.json` directly. Either the docs
   overstate multi-id support, or `run wait` should accept a
   whitespace-separated list in one argv token (or a `--runs-file`). Right now the
   documented batch-wait primitive silently no-ops under a very common invocation
   shape.

**Aggregate impact for this session:** none of the 4 stuck runs lost committed
work (2 had done nothing → re-spawned clean with fresh `-r2` idempotency keys;
2 were recovered from git), but recovery required ~4 rounds of manual
git-verification + worktree cleanup + re-spawn, and the batch could not be
trusted to `run wait`. The recurring pattern is: **a dead/absent supervisor
leaves a run `pending` with no terminal transition, so the caller cannot
distinguish "still running" / "silently failed to spawn" / "done but not
terminalised" without cross-checking git per run.** Fixes #1 (fail loudly), #5
(no false terminal with empty children), and a symmetric "supervisor death after
start → `failed`, never stuck `pending`" would let a fan-out driver trust run
status again.

### Environment (2026-07-24 repro)
- orchestratectl 0.1.0 (commit a54f0ff6)
- macOS (Darwin 25.5.0)
- Repo: 3dbear-monorepo; heavy parallel-session day again; 8 fan-out children +
  2 spinoffs spawned within ~40 min, main moving under the stint via other
  sessions. Failure rate this batch: 4/11 runs stuck (1 Signature A, 1 Signature
  B, 2 done-but-not-terminalised).
