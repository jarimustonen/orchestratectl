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
`retry_at` has elapsed:

1. Re-read the node under the run lock. If it is now terminal or a real report landed →
   drop the park (nothing to do).
2. **Re-verify empty-handed** (`git_ahead_count == Some(0)`) under the lock. If commits
   appeared (a late-committing agent) → drop the park and let the next watchdog tick
   terminalize-with-recoverability. This is the retry⟂salvage race guard: a retry can
   never clobber a branch that gained commits.
3. Tear down the stale worktree + branch + tmux window (empty-handed → `git branch -d`
   safe; guarded by the same source-relative unmerged check as `cleanup_node`, so a
   non-empty branch is never deleted).
4. `run::spawn::run_create_sh_with_tmux_retry` with a fresh branch name (`<base>-r<N>`),
   `--base <source_branch>`, the run's original `prompt.md`, and (for headless runs) the
   managed tmux session. Verify the new pid is alive.
5. **Success** → emit `node.retry` (above). Drop the park.
   **create.sh failure** → bump an in-memory spawn-failure counter and reschedule
   `retry_at` with backoff; once the in-memory spawn-failure budget is exhausted →
   synthesize the terminal `failed` report and drop the park (infra genuinely broken).

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
