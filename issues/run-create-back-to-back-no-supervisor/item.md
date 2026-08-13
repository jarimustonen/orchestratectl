---
created: 2026-07-25
updated: 2026-08-13
type: bug
status: open
priority: normal
labels: [defer-0.2.1]
---

# Second of two back-to-back 'run create' calls left supervisor-less (pid null, no worker node)

## Description

Observed during a /stint round (kunnollavauhtiin-monorepo, orchestratectl 0.1.0). Two 'orchestratectl run create --kind spinoff --headless ... --output jsonl' calls were issued back-to-back in one shell (create A; echo ===; create B).

Observed: create A printed its success envelope (supervisor pid set, alive). create B's envelope was NEVER printed to stdout, yet the run WAS created — it appeared in 'run list' with status pending, node_count 0, and supervisor {pid: null, alive: false}. 'run show B' confirmed no supervisor and no worker node. 'orchestratectl run reattach B' spawned the supervisor (new pid) and the worker node appeared; the run then proceeded normally to done+merged.

Expected: either 'run create' fully spawns the supervisor before returning (and prints its envelope), or it fails loudly. A silently supervisor-less pending run is confusing — it looks stuck and only 'run reattach' (non-obvious) recovers it.

Uncertain whether root cause is a race between rapid successive 'run create' calls or an artifact of backgrounding both in one shell; reattach recovery worked cleanly. Filing as low-severity friction with the exact repro. Env: macOS, headless session (detached 'headless' tmux).

## Comments

### 2026-07-25T09:41:39Z · @claude

Investigated; no code-level race found in rapid successive `run create`. Each call mints
a fresh ULID run id into its own run dir; the only shared state (`ensure_root`, the
idempotency store keyed by (repo,branch,key)) is lock-safe and per-key. The reported
symptom (second call's envelope never printed to stdout, run created with pid null / no
worker node, recovered only by `run reattach`) matches the same root as
`supervisor-spawn-fails-silently-at-run-create`: `create.sh` was slow/blocked so the
caller's process was interrupted before `node.created` + the supervisor spawn — the
reporter themselves flagged "artifact of backgrounding both in one shell." The subsequent
`run reattach` recovering a worker node is consistent with the original backgrounded
`run create` finishing `create.sh` after the reattach.

What the creation-path guards now give this scenario (so it fails loudly / recovers
deterministically rather than looking silently stuck):
- If the supervisor does not confirm, `run create` returns `supervisor_spawn_failed`
  with the run id instead of a silent supervisor-less `pending`.
- A reattached supervisor for a run whose worker was never created terminalizes it
  `failed` (no-worker guard) rather than presenting an ambiguous `pending`.
- Idempotency keys are now stored BEFORE the supervisor spawn, so a keyed retry replays
  the same run instead of creating a duplicate.

Left OPEN: no reproducible code race was isolated, so there is nothing more to fix here
without a live repro. If it recurs, capture whether the two `run create` processes were
backgrounded in one shell (`create A & create B &`) vs. run sequentially — the former is
the suspected cause and is a shell-usage issue, not an octl race.

### 2026-07-28T08:57:27Z · @jari

Confirmed again 2026-07-28 (3dbear-monorepo /stint, orchestratectl 0.1.0). Three back-to-back 'run create --kind spinoff --headless' in a single Bash loop: only the FIRST printed its envelope before the shell 2-minute timeout fired (Exit 143). The second appeared in run list as pending, node_count 0, supervisor pid null — same supervisor-less zombie. Cancelled with 'run cancel' (clean, worktree_root null so no git state), re-spawned individually with explicit 240s timeout — worked every time. Reliable workaround: one 'run create' per Bash call (not a loop), each with its own generous timeout.

### 2026-08-05T00:00:00Z · @jari

New datapoint that widens the trigger beyond "back-to-back in one shell" (3dbear-monorepo, orchestratectl 0.1.0). A **single, first** `run create --kind spinoff` (no loop, no other create in the same call, foreground) hit the Bash 2-minute timeout (Exit 143) mid-setup and left the exact zombie: `manifest.json` written, `supervisor {pid: null, alive: false}`, `node_count: 0`, events.jsonl containing only `run.created`, and NO `supervisor.stderr.log` at all. So the shared cause is simply "`create.sh` did not finish before the caller was interrupted" — a busy host is enough; concurrency is not required.

Two things worth noting against the earlier resolution comment:
1. The stated guard ("if the supervisor does not confirm, `run create` returns `supervisor_spawn_failed` with the run id instead of a silent supervisor-less pending") did **not** fire here — because the caller process was *killed* by the external timeout before it reached that return path. When `run create` is itself interrupted, nothing on the octl side gets to emit the loud failure. The zombie is indistinguishable from a healthy `pending` except by `supervisor.alive: false` + absent stderr log.
2. Recovery via `run cancel` was clean (worktree_root null → no git state to unwind), then re-create with an **idempotency key in a backgrounded Bash call** (so the external timeout can't interrupt it) spawned the supervisor normally within ~80s.

Suggested direction (still no in-process race to fix): make the zombie self-healing or self-evident without operator archaeology — e.g. a `pending` run whose manifest has no supervisor and whose age exceeds the startup timeout could be reaped/failed by the next `run list`/`run show`, or `run create` could fork the supervisor detached and return fast so the caller's own timeout can't straddle the spawn. The current reliable workaround for agents: run `run create` **backgrounded** (not foreground) so a harness/Bash timeout never interrupts setup.

## Decisions

### 2026-08-13T11:10:30Z · @adr-decision-2

DEFER-to-0.2.1: Supervisor-existence bucket — resolved by the lease. The clean answer is the pi.dev self-report/lease plugin (0.2.1), not the 0.2.0 thin core. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
