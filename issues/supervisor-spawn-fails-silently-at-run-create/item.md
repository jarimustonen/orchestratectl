---
created: 2026-07-22
updated: 2026-07-23
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
