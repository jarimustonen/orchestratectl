# Design — bounded auto-retry on agent-died for autonomous single-node workers

## Problem

An autonomous single-node worker (spinoff / research / bugfix / technical-decision /
make-skill) can die `agent-died` intermittently (confirmed transient — see
`worker-process-hang`). Today the watchdog synthesizes a terminal `failed` report on
the FIRST death; recovery is a manual re-spawn. We want a **bounded** auto-retry: on an
**empty-handed** death (branch level with source, nothing committed), re-spawn a fresh
worker at the run's source branch up to `RETRY_MAX_ATTEMPTS`, with backoff, before
finally terminalizing `failed`.

## Precedence — retry ⟂ salvage (exact interaction with `agent-death-strands-recoverable-work`)

The synthesis path already computes a recoverability signal when the dead branch carries
commits ahead of source. The two paths are **mutually exclusive**, keyed on
`git rev-list --count <source>..<branch>`:

- **`> 0` (committed work) → SALVAGE wins.** Terminalize `failed` with the
  `recoverable_work` block exactly as today. NO retry (a retry from base would abandon a
  fresh worktree, but the committed work still needs a human/salvage to land — the
  operator owns it). Unchanged behaviour.
- **`== 0` (positively empty-handed) → RETRY (autonomous single-node worker only).**
- **`None` (git error / missing input) → terminalize as today.** We never retry unless we
  can POSITIVELY prove empty-handed; a git hiccup declines to fabricate a "safe to discard"
  verdict.

## Bounded counter — durable, restart-safe

The authoritative attempt count lives in the **node projection**: a new
`Node.retry_attempts: u32` (`#[serde(default)]`, replay-safe). `reduce_node.retry`
increments it. The retry decision at death-detection reads `retry_attempts`:

- `retry_attempts >= RETRY_MAX_ATTEMPTS` → terminalize `failed` (exhausted; stamp
  `retry_attempts: N` on the report).
- else → schedule attempt `retry_attempts + 1`.

Because the bound is a persisted projection field, it survives a supervisor restart: a
crash while parked (before `node.retry` is emitted) leaves the count unchanged and the
node re-enters the same attempt on restart — never an over-count, never an unbounded loop.

## Two events + one in-memory park (mirrors the child-supervisor state machine)

1. **In-memory park (`retry_states: BTreeMap<String, RetryState>`).** Threaded into
   `watchdog_tick` like `half_state_streak`. On an empty-handed autonomous death with
   attempts remaining, the node is parked `RetryPending { attempt, retry_at: now +
   backoff(attempt) }` and NOT terminalized. The first-pass candidate scan **skips a
   parked node**, so the dead pid is not re-detected while it waits out the backoff.
   Ephemeral — the durable truth is `retry_attempts`; on restart the watchdog simply
   re-detects the dead pid and re-parks (no reseed needed).

2. **`node.retry` durable event** (attempt N + `reason` + fresh spawn metadata).
   Emitted by the reconcile pass AFTER a successful re-spawn. Its reducer
   (`reduce_node_retry`) rewires the node projection to the new agent (branch, base_sha,
   worktree_path, tmux identity, agent_pid), sets `status = Pending`, `started_at = ev.ts`
   (fresh spawn-grace), clears the (empty) `last_report`, and increments `retry_attempts`.
   Terminal-guarded (a settled node is never resurrected). Unknown to old logs → replay
   unchanged.

## Reconcile pass (`reconcile_agent_retries`)

Runs each tick (inside `watchdog_tick`, after the candidate loop) for parked nodes whose
`retry_at` has elapsed. **Spawn-before-teardown ordering** (revised per the /llm-review —
see `history/review-autoretry-agent-died-worker.md`): the stale worktree is never destroyed
until a replacement is durably attached.

1. Re-read the node under the run lock. If terminal, not retry-eligible, or no longer
   positively empty-handed → drop the park. Capture the node + manifest; drop the lock.
2. **Spawn the fresh worker FIRST** — `run::spawn::run_create_sh_with_tmux_retry` with a
   fresh branch name (`<stem>-r<N>`), `--base <source_branch>`, cwd `<source_repo>`, the
   run's `prompt.md`, and (headless) the managed tmux session; verify the new pid is alive.
   On **create.sh failure** the stale worktree is untouched (so the empty-handed re-verify
   still holds and the budget is real): bump the in-memory spawn-failure counter and
   reschedule; once the budget is exhausted, `terminalize_respawn_failure` (which returns
   `bool` — the park is removed ONLY once the terminal report is durably recorded, so a
   lock/append failure re-fires instead of un-tracking a non-terminal node across a restart).
3. Re-acquire the lock. **Re-verify the stale node is still non-terminal AND still
   `node_is_empty_handed`.** A `run cancel` (terminal) or a late commit (salvage territory)
   aborts the retry: the fresh spawn is torn down (`teardown_respawn_outcome`) and the stale
   node is left for the terminal/salvage path. This is the retry⟂salvage guard — a retry can
   never rewire away from, or clobber, a branch that gained committed work.
4. Emit the durable `node.retry` event (rewires the node to the fresh agent, increments
   `retry_attempts`). If the append fails or the lock can't be taken, tear down the fresh
   spawn and leave the park to re-fire cleanly on the same `-rN` name.
5. **Only now** tear down the stale worktree + branch + tmux window via `cleanup_node`
   (whose own source-relative guard + `git branch -d` backstop PRESERVE rather than delete
   if commits somehow appeared — never destroying committed work). Drop the park.

`node_is_empty_handed` requires BOTH zero commits ahead of source AND a **clean worktree**
(no staged/modified/untracked files), so a dead agent's uncommitted scratch is never
force-removed by the retry — such a death falls through to the preserve-on-blocked terminal
path. The park is only ever created for the strong `Liveness::Dead` verdict (the PID is
provably gone), never the weaker `Recycled` / `TmuxGone` half-states where a still-live agent
could be committing.

`teardown_respawn_outcome` kills the new agent, removes its worktree, force-deletes its
freshly-minted `-rN` branch, and closes its tmux window — so an unattached spawn never leaks
a live token-burning agent and never poisons the branch name for the next attempt.

## Eligibility gate

Retry fires only when ALL hold:

- `n.kind.lifecycle() == Autonomous` (interactive → a human drives; never retry).
- `n.kind.is_autonomous_single_node_worker()` — Spinoff / Research / TechnicalDecision /
  MakeSkill / Bugfix. Excludes FanOut/Orchestrate (drivers — their driver node has no
  `agent_pid`, so it never reaches synthesis anyway) and Orchestrated (a DAG child whose
  parent supervisor owns its retry policy).
- `n.parent_node_id.is_none()` (top-level worker).
- positively empty-handed (`git_ahead_count == Some(0)`).

## Constants

- `RETRY_MAX_ATTEMPTS = 3` (matches the observed "third spawn landed" evidence).
- `RETRY_BASE_BACKOFF = 10s`, `RETRY_MAX_BACKOFF = 120s`, exponential, clamped —
  `retry_backoff(attempt)`. Env override `OCTL_AGENT_RETRY_BACKOFF_SECS` (tests set `0`).

## Invariants preserved

- Every append goes through the `LockedRun` witness / `append_and_apply_*`.
- Teardown preservation gates untouched — retry only ever tears down a *provably
  empty-handed* branch, re-verified under the lock immediately before removal.
- Healthy runs (no death) never enter this path.
- New event kind is additive; old logs reduce byte-for-byte unchanged.
</content>
</invoke>
