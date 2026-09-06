---
created: 2026-07-26
updated: 2026-07-26
type: feature
status: done
priority: normal
related: ['@worker-process-hang']
closed: 2026-07-26
---

# Capture autonomous agent pane output to durable `<run-dir>/agent.log`

_Source: crates/taskfleet-cli/src/supervise_

Companion to [[worker-process-hang]]. An autonomous worker's stdout/stderr goes
ONLY to its tmux pane, which the supervisor kills on cleanup — so a genuine
death (hang / API cutoff / crash) left **zero trace** of the cause. This made
the ~13-min deterministic `agent-died` on `pipeline-tiered-triage` (see
`worker-process-hang` 2026-07-26 corroboration) uninvestigatable post-mortem.

## What landed

The supervisor now tees each worker node's tmux pane to `<run-dir>/agent.log`
via `tmux pipe-pane`, armed once the node's `tmux_identity` is first observed.

- New module `crates/taskfleet-cli/src/supervise/capture.rs`; `capture::capture_tick`
  is called each supervisor tick (BEFORE the watchdog, so startup output inside
  the spawn-grace window is captured too).
- New `RunPaths::agent_log()` → `<run-dir>/agent.log`. The file lives in the RUN
  DIR, not the worktree, so it survives `git worktree remove` + `tmux
  kill-window` on teardown. Cleanup never touches run-dir files; a regression
  test (`cleanup::tests::agent_log_survives_teardown`) pins this.
- Works for headless (autonomous) sessions — the socket is read from
  `tmux_identity.socket`, which create.sh records for the detached session.
- Best-effort + non-fatal: a failed `pipe-pane` (old tmux, missing identity,
  server down) logs a warning and continues; never blocks the spawn or the
  liveness path.

## Design decisions / deviations from the task sketch

- **Plain `pipe-pane` (no `-o`), not `-o`.** The task sketched `pipe-pane -o`.
  Per `man tmux`, `-o` is a *toggle*: an existing pipe is closed and NOT
  reopened. Calling it repeatedly would toggle capture OFF. We use plain
  `pipe-pane -O` ("close any existing pipe, open a fresh one", explicit output
  direction) and guarantee once-per-node via a persisted armed set.
- **Armed set persisted in `SupervisorState.captured_armed`.** The capture pipe
  (`head -c … >> agent.log`) is a child of the tmux SERVER, not the supervisor,
  so it survives a supervisor restart. Persisting "already armed" means a restart
  does NOT re-run `pipe-pane` — the live pipe keeps appending with no
  close/reopen transition gap. (All `SupervisorState` fields are
  `#[serde(default)]`, so old state files load fine.)
- **Bounded retry on transient failure.** A node is marked armed only on a
  *successful* `pipe-pane`; a transient failure (tmux still coming up during
  spawn grace — exactly the node whose startup output we most want) is retried
  on later ticks up to `MAX_CAPTURE_ATTEMPTS` (10), then given up so a
  permanently-broken tmux is not re-probed forever. Retry counters are in-memory.
- **Every tmux shell-out is time-bounded** via the shared
  `crate::proc::run_with_timeout` (the watchdog's runner), 5s. capture runs
  before the watchdog in the single-threaded loop, so a wedged tmux server must
  never stall the tick.
- **Size cap ADDED (`head -c 64 MiB`).** Once the cap is read `head` exits, tmux
  closes the pipe on EOF, capture stops — bounds disk use without disturbing the
  agent. Generous enough to hold a full heavy-LLM run's pane output.
- **Pane targeting caveat (documented, spun off).** `pipe-pane -t <window_id>`
  targets the window's *active* pane. For the autonomous headless path (the
  priority) the window has exactly one pane = the agent, so this is correct. An
  interactive user who splits the window could shift the active pane; capturing
  by a stable `pane_id` recorded at spawn is [[capture-agent-pane-by-pane-id]].
- **One `agent.log` per run dir.** Matches the task's deliverable. Each
  autonomous worker is its own run with its own run dir + supervisor (one worker
  node per run), so the file has a single writer in practice.

## Review

`/llm-review` (Gemini 3.1 Pro, GPT-5.6-sol, Opus 4.7) → `/assess-findings`:
5 FIX + 1 SPIN-OFF, 0 DROP. All 5 FIX applied (timeout, bounded retry, size cap,
explicit `-O`, persist armed set). SPIN-OFF = the pane-id targeting
([[capture-agent-pane-by-pane-id]]). Artifacts:
`history/review-capture-agent-output-to-run-dir.md`,
`history/assessment-capture-agent-output-to-run-dir.{json,md}`.

## Tests

- `supervise::capture::tests::*` — argv construction (socket present/absent,
  `-O`, `head -c` cap), shell-quoting, lenient spawn-failure, **timeout on a
  wedged tmux**, **bounded retry then give-up**, and end-to-end scan→dispatch
  through a stubbed tmux: armed once, targets the run-dir `agent.log`, second
  tick is a no-op, terminal node skipped, no-identity node left for retry.
- `supervise::state::tests::*` — round-trips `captured_armed`; loads a legacy
  state file without the field.
- `supervise::cleanup::tests::agent_log_survives_teardown` — the run-dir log
  survives the most destructive teardown (explicit-merge force-remove + `-D`).
