---
created: 2026-08-17
updated: 2026-08-17
type: bug
status: open
priority: high
labels: [run-wait, supervisor, reliability]
---

# run wait reports a healthy slow-starting run as stillborn (false verdict, causes duplicate spawns)

## Description

`orchestratectl run wait <id>` returned **immediately** (`waited_ms: 0`) with a
stillborn verdict for a run that was merely **slow to create its worker node**.
The run recovered seconds later and ran to completion normally.

This is the inverse of the (now fixed) `run-wait-stillborn-run-not-detected`:
that one *missed* a real stillborn; this one *invents* one. The stillborn
detection added in `3567d75` / `a44e567` appears to latch its verdict without
requiring that the run has had a plausible grace period to create `n-0001`.

## Observed (2026-08-17, real stint)

Spawn of a `--kind spinoff --headless` run `01m077het6e06zhmeefa4bsz9k`:

```json
{"run_id":"01m077het6e06zhmeefa4bsz9k","status":"pending","merged":false,
 "landed":false,"landed_method":"unverified","stalled":true,
 "attention_required":false,
 "error":"supervisor died before creating any worker node"}
```

`waited_ms: 0` — the verdict was rendered on the first poll. `run show` at that
moment agreed: `node_count: 0`, `stillborn: true`, `supervisor.pid: null`,
`supervisor.alive: false`.

**Seconds later the same run was healthy:** `node_count: 1`,
`stillborn: false`, `stalled: false`, `supervisor.alive: true`, with a live
tmux window (`headless:6`) and a worktree. It went on to work the issue normally.

So the "supervisor died before creating any worker node" verdict was **false**;
the supervisor had simply not recorded its pid / created its node yet at the
instant `run wait` sampled it.

## Impact — this is the dangerous part

The false verdict directly causes **duplicate spawns of the same work**. Acting
on the stillborn report, the caller re-spawned the same issue; both runs then
created worktrees and both supervisors came alive. Two autonomous agents were
briefly poised to implement the same issue in parallel, which risks:

- two workers racing to merge the same change (conflicting/duplicate commits),
- wasted spawn + LLM cost,
- and, in the general case, an agent hand-salvaging "stranded" work that was
  never actually stranded.

The caller only avoided the duplicate by manually cross-checking `node_count`
and `tmux list-windows` for both runs and cancelling the loser. An autonomous
orchestrator following the documented "trust the CLI's flags" guidance would
have proceeded into duplicate work.

Aggravating detail: the **cancelled** run also later showed `node_count: 1` and
`supervisor.alive: true` after cancellation, so post-cancel state is likewise
ambiguous when read too soon (its tmux window was correctly absent, and
`run cancel` did remove its worktree).

## Repro sketch

Spawn a `--kind spinoff --headless` run and call `run wait` (or `run show`)
essentially immediately, before the supervisor has recorded its pid and created
`n-0001`. On a busy machine the create→node window is wide enough to hit
routinely. Observed here with `harness: pi`.

## Expected

A stillborn verdict must not be reachable from "the supervisor has not recorded
itself *yet*". Suggested directions:

- Require a **minimum grace period** since `created_at` (and/or a bounded number
  of consecutive confirming polls) before `run wait`/`run show` may report
  stillborn. A run whose `created_at` is seconds old should read as
  *starting*, not *stillborn*.
- Distinguish **`starting`** from **`stillborn`** in the surfaced state so a
  caller can tell "not up yet" from "will never come up".
- Do not latch the stillborn verdict irreversibly; re-evaluate it if the run
  subsequently gains a node or a live supervisor (the latching hardening in
  `a44e567` may be what makes the false verdict sticky).
- Document, in the stint/worktree guidance, that a stillborn verdict on a
  freshly created run should be re-checked before re-spawning.

## Acceptance criteria

- [ ] A healthy run that is slow to create `n-0001` is never reported stillborn
      by `run wait` or `run show`
- [ ] Stillborn detection requires a grace period and/or repeated confirmation,
      not a single early sample
- [ ] `starting` is distinguishable from `stillborn` in the output
- [ ] A stillborn verdict is re-evaluated (not latched) if the run later gains a
      node or live supervisor
- [ ] Regression test covers the create→node race window

Relates to: `run-wait-stillborn-run-not-detected` (fixed; the inverse failure),
`run-create-long-title-stillborn` (fixed; a *real* stillborn class),
`supervisor-pid-claim-race`.
