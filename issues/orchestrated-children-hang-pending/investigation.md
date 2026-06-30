# Investigation — orchestrated children hang in `pending`, no teardown

## Root cause (hypothesis #1 confirmed; #2 and #3 ruled out)

**An `--kind orchestrate` driver run spawns no supervisor process, so its
parent-pointed `--kind orchestrated` children are never adopted, their
terminal `node.report` is never consumed, the child run is never rolled up
to a terminal status, and its worktree + tmux window are never torn down.**

The supervisor — not `run merge`, not `run cancel` — is the canonical actor
that rolls a run up to a terminal `run.status` and tears down its worktree
(state-integrity invariant 5; `crates/octl-cli/src/supervise/cleanup.rs`
`rollup_status` + `cleanup_terminal_nodes`). A child run delegates supervisor
creation to its parent's supervisor (single-arbiter, design §7.2):

- `crates/octl-cli/src/run/create.rs:544-548` — for a child spawn
  (`is_child`), `run create` deliberately does **not** spawn a supervisor;
  the parent supervisor sees `child.spawned` and forks the child supervisor.
- `crates/octl-cli/src/supervise/mod.rs:325-415` — that adoption only happens
  inside a running parent supervisor's own-run tail loop.

But the orchestrate **driver** never gets a supervisor:

- `crates/octl-cli/src/run/create.rs:182-184` force-skips materialization for
  `Kind::Orchestrate` (the orchestrator agent runs in the user's main
  conversation; there is no worktree to boot).
- `crates/octl-cli/src/run/create.rs:414-442` — the `skip_materialize`
  short-circuit returns early with `supervisor_pid: None`. The non-skip
  supervisor-spawn at `create.rs:544-548` is never reached.
- `crates/octl-cli/src/run/create.rs:736-738` then labels the envelope's
  `supervisor` field `"orchestrator-in-main-conversation"`.

Net effect for every orchestrated child of that driver:

1. The child materializes, emits `node.created` on its own log (so the child
   manifest *does* register `n-0001` — see below on `nodes: []`), and emits
   `child.spawned` on the **driver's** log.
2. Nothing is tailing the driver's log — there is no driver supervisor — so
   no child supervisor is ever forked.
3. The child's closing `orchestratectl run merge` lands the merge and appends
   a terminal `node.report` on the child's log (`merge.rs:185`), which the
   reducer folds to terminalize the child's `n-0001` node. But `run merge`
   never writes `run.status` and never tears down — those are the
   supervisor's job (`merge.rs` doc comment; cleanup invariant 5).
4. With no supervisor, `rollup_status` never runs → the child run stays
   `status: pending`; `cleanup_terminal_nodes` never runs → the worktree +
   tmux window linger; `run wait <child>` (which polls `manifest.status`,
   `wait.rs`) blocks until its timeout — previously **forever**, since
   `--timeout` defaulted to none.

This also explains the recovery the reporter described: `run cancel <child>`
appends `run.status: cancelled` via `octl_core::cancel_run` (`cancel.rs:46`)
but performs **no teardown** — teardown is, again, the supervisor's job, and
there was no supervisor. By contrast `--kind spinoff` children that the
reporter "re-ran" were top-level (no `--parent-run-id`), so each spawned its
own supervisor at `create.rs:544-548` and tore itself down — the working kind.
`--kind fan-out` campaigns work for the same reason: the fan-out *driver* is
not in the skip-materialize set, so it boots a worktree **and a supervisor**
that adopts the fan-out children. The orchestrate driver is the only top-level
kind that skips supervision — that is the defect.

### Hypothesis #2 (manifest never registered the node) — not a binary bug

`run create` emits `node.created` for a materialized child unconditionally
(`create.rs:508-529`), and the manifest's `node_count` is re-derived from the
`nodes/` projection dir on every applied event (`events.rs:739`,
`derive_counters`). The reporter's `nodes: []` is most plausibly an imprecise
restatement of "the run never went terminal" (`run show`'s `nodes` is a count
of node JSON files, `show.rs`), or an artifact of inspecting the run before
`node.created` settled. The new `e2e_orchestrated` test asserts the worker
node **is** registered (`node_count == 1`) after teardown, closing this out.

### Hypothesis #3 (agent merged manually) — not a doc bug

The bundled `worktree-orchestrated` SKILL's closing recipe correctly invokes
`orchestratectl run merge` (SKILL.template.md §"Terminal report", lines
167-225) — not a raw `git merge`. The merge + `issuectl` link commits the
reporter saw are consistent with `run merge` having run. The doc is right; the
run still hung because no supervisor consumed the report it submitted.

## Fix

1. **Spawn a supervisor for the production orchestrate driver**
   (`create.rs`, `skip_materialize` branch). The driver has no worker node for
   the watchdog to adjudicate, but it now tails its own log, forks a child
   supervisor per `child.spawned`, and consumes each child's `node.report` for
   DAG aggregation — exactly the machinery fan-out already relies on. Each
   forked child supervisor then rolls its child up to `done` and tears down the
   child's worktree + window. The test-only skip hatches
   (`--skip-materialize`, `OCTL_TEST_SKIP_MATERIALIZE`) still produce a pure
   skeleton with no supervisor.
2. **`run cancel` teardown** falls out of (1) for free: with a live child
   supervisor, a cancel that pushes the child terminal is picked up on the next
   supervisor tick, which runs `cleanup_terminal_nodes` (orchestrated is an
   `Autonomous` lifecycle, so the cleanup gate fires on any terminal). No
   side-channel cleanup is added — the supervisor stays the single teardown
   actor (invariant 5).
3. **Defensive `run wait --timeout` default** (Option A): `run wait` now
   defaults `--timeout` to `6h` so a genuinely-stuck run surfaces as an exit-2
   timeout instead of blocking an orchestrator indefinitely.
4. **SKILL doc**: the orchestrate SKILL's "you are the supervisor" note is
   corrected (a supervisor process now owns child lifecycle + teardown; the
   orchestrator agent owns planning), and a final step closes the driver run so
   its supervisor winds down at campaign end.
