# Pi headless spinoff batch stillbirth: root cause

## Evidence

The three affected run logs each contain exactly one event: `run.created` at
`2026-08-17T03:58:35Z`. They contain no `node.created`, `supervisor.started`,
or `supervisor.stderr.log`. The two peers from the same batch also begin with a
`run.created` event at that instant, then record `node.created` 12 seconds and
26 seconds later respectively.

That rules out a supervisor launch race and a Pi worker failure after launch:
the old create process never reached either `node.created` or supervisor spawn.
It was still blocked in the synchronous `create.sh` materialization call when
its client timed out. `run_create_sh` uses `Command::output`, so it does not
return an outcome until workmux, tmux, and harness agent discovery all finish.
The field evidence cannot distinguish the condition that prolonged that call
(PTY/tmux contention, admission pressure, or slow Pi startup): its captured
stderr was discarded when the client was terminated. `source_repo`,
`source_branch`, and `worktree_root` being null are not discriminating evidence:
the two successful peers have the same omitted optional source fields.

## Root cause

`run create` previously published `run.created` before its blocking,
interruptible materialization step. A caller-side timeout or hard cancellation
therefore left a public `pending` manifest with no node. The command never made
its own success claim, but downstream orchestration treated the durable
skeleton as an accepted run. This is an atomicity defect in orchestratectl's
create protocol, independent of the still-unidentified saturation trigger.

## Fix

Creation now writes the prompt, event log, manifest, and `node.created`
projection under `~/.orchestratectl/.creating/runs/<run-id>/`. Only after
`create.sh` has returned a live PID and `node.created` is durable does it
atomically rename that directory into `~/.orchestratectl/runs/<run-id>/`.
The parent `child.spawned` event, idempotency reservation commit point, and
supervisor launch all follow publication. Thus a successful create names an
existing worker node, and an interrupted materialization cannot expose a
0-node run to `run list`, `run wait`, or a parent supervisor.

A hard kill can leave private staging state, and may leave workmux-side
resources when it also kills the shell before its cleanup trap runs. That is
operational debris, not a public run or a false success claim. It is retained
for diagnosis rather than guessed at or auto-cleaned in a correctness-sensitive
path. The prompt remains the exact file passed to create.sh and is copied by
its existing worktree source contract.

## Regression coverage

`crates/octl-cli/tests/creation_reliability.rs::interrupted_create_never_publishes_a_zero_node_run`
deterministically blocks a fake `create.sh`, kills `run create` as a timed-out
client would, and proves that `runs/` remains empty while the unfinished state
is private under `.creating/`. It also proves that a keyed retry fails with
a bounded `idempotency_publish_timeout` rather than falsely replaying private
state as a success. Concurrent duplicate callers wait for the original creator
to publish before replaying. This covers the atomicity seam without a flaky PTY, tmux, or Pi
startup soak.

## Remaining larger work

A hard kill before publication leaves the existing early idempotency reservation
without an owner lease, so it cannot be safely reclaimed while another creator
might still be alive. Likewise, publishing a child and appending its
`child.spawned` event touch two run logs and need a recoverable cross-run
transaction. Both are recorded in follow-up
`create-idempotency-lease-recovery`; this issue remains open because that work
is required to make retry recovery fully automatic. The landed staging boundary
nevertheless closes the reported false-success/stillborn manifestation.
