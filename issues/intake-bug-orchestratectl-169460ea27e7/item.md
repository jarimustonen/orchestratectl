---
created: 2026-08-16
updated: 2026-08-17
type: bug
reporter: jari
status: open
priority: normal
labels:
- via:agent-homebase-wrapup
lane: lifecycle
---

# stale pending runs clutter run list and look like live workers

## Description

stale pending runs clutter run list and look like live workers

`orchestratectl run list` accumulates runs stuck at `status: pending` that never materialized —
no repo, no worktree, no branch. They are indistinguishable at a glance from runs that are
genuinely in flight.

Observed (2026-08-16), during a stint preflight whose whole purpose was "confirm no worker is
still unsettled before wrapping up":

    $ orchestratectl run list --output json | jq -r '(.data.runs // .data)[]
        | select(.status=="pending" or .status=="running") | "\(.run_id) \(.status) \(.title)"'
    01m05tqq6g0ekacb2bm6gw3h0t pending  cli-canon-config: config path / config show --json
    01m05tmcnn7kfs0cye2kq1a7nb pending  cli-canon-config
    01m05swgs6516qhs2mcn589295 pending  long-title-stillborn
    ... 10 total, spanning several repos

Two of them carried a title matching an issue slug that had just been closed in the repo being
wrapped up, which read as "duplicate work is about to merge into my main". Ruling it out meant
reading manifests by hand:

    $ jq -c . ~/.orchestratectl/runs/01m05tmcnn7kfs0cye2kq1a7nb/manifest.json
    {"status":"pending","source_repo":null,"worktree_root":null,"harness":"pi","node_count":1,
     "created_at":"2026-08-16T17:41:38Z", ...}

`source_repo: null` + `worktree_root: null` = stillborn; it can never merge anything. Several
titles look like test fixtures (`long-title-stillborn`, `space-custom-template`,
`all-kinds-spawn`), so at least some are likely test-suite residue.

Expected, any of:

- a run that never got a worktree/repo reaches a distinct terminal state (`stillborn` /
  `abandoned`) instead of resting at `pending` forever;
- `run list` defaults to hiding runs that never materialized, or exposes a filter
  (`--active`, `--repo <path>`) so a caller can ask "is anything live for THIS repo";
- failing that, `run list` surfaces `source_repo` / `worktree_root` in its default output so
  stillborn rows are visible without opening manifests.

Why it matters: "has every run settled?" is a precondition for wrapping a work session and for
deciding whether to re-spawn work. Right now answering it correctly requires per-run manifest
inspection, and answering it naively gives a false "workers still running" — or, worse, a false
belief that duplicate work is inbound.

Side observation, possibly related and possibly its own bug: `orchestratectl node show n-0001
--run-id <id> --output json` produced no output at all for a completed run, while
`~/.orchestratectl/runs/<id>/nodes/n-0001.json` existed and was readable.

## Comments

### 2026-08-17T08:14:16Z · @orchestrator

Admitted to the plan 2026-08-17 (needs-triage removed) and RE-SCOPED. This round's `pi-spinoff-batch` fix (staged run creation, released in 0.2.2) should stop NEW stillborn pending runs from being published at all, so the prevention half is done. What remains and is what this issue now covers: (a) the ~7 already-accumulated stale pending runs on disk, and (b) the listing/presentation side — a stale pending must be distinguishable at a glance from a live worker in `run list`. Corroborated again this session: `run list` returned ~301KB dominated by old pendings, several belonging to other repos, which is the signal /stint-start's preflight reads.
