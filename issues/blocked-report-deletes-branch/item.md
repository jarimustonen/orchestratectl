---
created: 2026-07-03
updated: 2026-07-03
type: bug
reporter: jari
status: fixed
priority: high
labels: [supervisor, data-loss]
commits:
- hash: fe44a56
  summary: preserve branch+worktree on blocked terminal report; -d safety net; tests
closed: 2026-07-03
---

# Blocked terminal report (success:false) deletes the worktree branch instead of preserving it for the human

_Source: supervisor teardown / node report path_

## Description

A single-worker run (observed with `--kind bugfix`) that submits a terminal
`node report` with `success: false` — the documented **blocked / needs-a-human**
path — has its worktree **and its branch** torn down by the supervisor. The
skill contract is that the blocked path must **leave the branch unmerged for the
human** to pick up; only the success/merge path (`run merge`) should delete the
branch. Result: the agent's committed work is lost from all refs — it survived
only as **unreachable git objects** (recoverable via `git fsck`); a `git gc`
would have destroyed it permanently. **Silent data loss.**

## Impact

- Severity: **high** — the blocked path exists precisely so a human can take
  over unfinished work; deleting the branch throws that work away.
- The loss is silent: `node report` returns success, the run winds down
  normally, and nothing warns that a branch with unmerged commits was deleted.
- Recovery is only possible before the next `git gc` and requires manually
  reconstructing the branch from `git fsck --unreachable` output — not something
  a user would know to do.

## Reproduction

1. Spawn a single-worker autonomous run: `orchestratectl run create --kind
   bugfix --title … --task …` (a `technical-decision` / `spinoff` almost
   certainly hits the same path — the shared blocked-report teardown).
2. In the worktree, make + commit real work on the run's `wt/<short>-<slug>`
   branch.
3. Submit the **blocked** terminal report:
   `orchestratectl node report "$run_id" "$node_id" --from-file report.json`
   with `{"success": false, "discussion_items": [ … ]}` (no `run merge`).
4. Observe: supervisor exits, the worktree dir is removed, **and the branch
   `wt/<short>-<slug>` is gone**. `git branch --list 'wt/<short>*'` → empty;
   the commits are only in `git fsck --unreachable`.

## Expected vs actual

- **Expected:** on a `success: false` terminal report, wind the run down (the
  tmux window may close) but **preserve the branch** (and ideally the worktree,
  or at minimum the branch) so the human can `git merge` / `/worktree-merge` it
  later. Branch deletion should be exclusive to the success/merge path.
- **Actual:** the blocked path tears down the branch the same as the success
  path, discarding the commits.

## Evidence (observed 2026-07-03)

- Run `01kwkvbh42hd5c0n6yfn4yjqmq`, `--kind bugfix`, slug
  `gertrud-health-snapshot-stale` (homebase repo).
- Agent committed two commits on `wt/01kwkvbh42-gertrud-health-snapshot-stale`,
  then submitted `node report` with `success: false` + a `discussion_items[]`
  entry (needed the user's sudo to finish).
- Events tail: `node.report` → `run.status` → `supervisor.exited`.
- Afterward: `git worktree list` had no gertrud entry, `git branch --list
  'wt/01kwkvbh42*'` was empty. The commits `e8d1a36…` and `22156bc…` were only
  present as dangling/unreachable objects; recovered manually via `git fsck
  --unreachable` + fast-forward back onto `main`.

## Contract references (what this violates)

The Claude Code skill docs shipped with orchestratectl describe the blocked
path as explicitly **non-destructive to the branch**:

- `worktree-bugfix`: *"Blocked / needs a human → nothing to merge … leaves the
  branch unmerged for the human to pick up."* and *"This records the node
  terminal without merging … the supervisor still winds the run down, but the
  branch is left unmerged for the user."*
- `worktree-technical-decision`: same — *"does NOT merge; it stops and submits a
  direct node report … The branch stays unmerged until the user breaks the tie
  and re-spawns."*

So either the teardown code is wrong (it deletes the branch on the blocked
path) or the docs are wrong; the docs describe the safe, intended behavior, so
the teardown should be fixed to match.

## Fix direction

- Gate branch deletion on the **terminal outcome**: delete the branch only on
  the merge/success path (`run merge`, `via: "explicit-merge"`), never on a
  `success: false` `node report`.
- Consider also preserving the worktree on the blocked path (or at least
  printing the branch name + "left for you to merge" so it's discoverable).
- Add a safety net: refuse to delete a `wt/*` branch that has commits not
  reachable from its source branch unless the merge path explicitly ran.

## Quick test

Reproduce steps 1–4 above; assert that after a `success: false` report the
branch still exists and `git log main..wt/<short>-<slug>` shows the agent's
commits.
