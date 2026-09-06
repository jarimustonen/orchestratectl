---
created: 2026-08-08
updated: 2026-08-10
type: bug
status: fixed
priority: normal
labels: [run-wait, supervisor, reliability]
commits:
- hash: 3567d75
  summary: run wait/show detect stillborn run as stalled
- hash: a44e567
  summary: harden stillborn detection per llm-review (latch verdict, docs, structured error)
closed: 2026-08-10
---

# run wait blocks full timeout on stillborn run (dead supervisor, 0 nodes, never started)

## Description

A `--kind spinoff` run was created successfully (`run create` returned a normal
success envelope with a supervisor pid), but the run **never spawned a worker
node**. Its manifest stayed at `status: pending` with:

- `supervisor: {pid: null, alive: false}`
- `node_count: 0`
- `worktree_root: null`
- `updated_at` == `created_at` (never progressed past creation)

`taskfleet run wait <id>` then **blocked for the full timeout (~6 h,
`waited_ms: 21600189`)** before returning, still reporting
`status: pending, landed: false, landed_method: unverified`.

This is a *stillborn* run — the supervisor died (or never took over) before
creating node `n-0001`. It is unambiguously stuck, yet nothing surfaces that:
`run wait` treats "pending with a dead supervisor and zero nodes" identically to
"pending, actively working."

## Observed vs expected

- **Observed:** `run wait` waits the entire timeout on a run that can never make
  progress (dead supervisor, 0 nodes). `run show` reports `stalled: false` and
  `status: pending` for it. The caller only discovers the problem by manually
  reading `manifest.json` (`supervisor.alive == false && node_count == 0`).
- **Expected:** `run wait` (and `run show`) should detect
  `supervisor.alive == false && node_count == 0 && updated_at == created_at` as a
  terminal **stalled/failed** condition and return promptly (non-zero under
  `--fail-on-error`), rather than blocking for the full timeout. At minimum
  `stalled: true` should be set so a caller can bail.

## Repro (observed 2026-08-07/08)

1. `taskfleet run create --kind spinoff --headless --title X --prompt-file …`
   → success envelope, supervisor pid returned.
2. Run never creates a node (root cause of the supervisor death not diagnosed;
   possibly resource contention — several spinoffs were spawned in the same wave).
3. `taskfleet run wait <id>` → blocks the full timeout, returns
   `{status: pending, merged: false, landed: false, landed_method: unverified}`.
4. `manifest.json` shows `supervisor.alive: false`, `node_count: 0`,
   `updated_at == created_at`.

Run id from the incident: `01kzd8xkhz70awwyg0vsdcrgdg` (kind spinoff, title
`emailhash-hmac-pepper`). The underlying work turned out to be already done
elsewhere, so no work was lost — but the stillborn run still consumed a full
`run wait` timeout and required manual manifest inspection to diagnose.

## Suggested fix

- In `run wait`'s poll loop, treat `supervisor.alive == false && node_count == 0`
  (with no forward progress since creation) as a terminal stalled state → return
  immediately, set `stalled: true`, and make `--fail-on-error` exit non-zero.
- Surface the same in `run show` (`stalled: true` for this shape).
- Optionally: a `run reattach` / `run cancel` hint in the returned envelope so the
  caller knows the run is dead rather than slow.

## Comments

Separate, milder rough edge seen the same session (probably NOT worth its own
issue): report-marker landings routinely return `landed: false /
landed_method: unverified` after the supervisor tears down, forcing manual
content-verification on the target branch. This is documented behavior in the
worktree-spinoff skill, so noting here only for context — the stillborn-run
timeout above is the actionable bug.
