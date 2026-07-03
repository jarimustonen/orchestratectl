---
created: 2026-07-03
updated: 2026-07-03
type: bug
status: in-progress
priority: high
---

# run create hard-wires create.sh 30s agent-startup timeout; unspawnable under load

_Source: crates/octl-cli/src/run/spawn.rs_

## Description

## Summary

`orchestratectl run create` does not expose or forward create.sh's
`--agent-startup-timeout` flag. create.sh accepts it (default 30s, range
1–600) and even documents "bump --agent-startup-timeout if the host is
loaded", but octl's `spawn.rs::run_create_sh` only forwards `--type`,
`--layout`, `--no-hooks`, `--keep-tmux-on-error`, `--parent-session`,
`--base` — never `--agent-startup-timeout`. So every spawn is hard-wired
to the 30s default.

## Impact

On a loaded host (observed 2026-07-03 on hauis, load avg 26–33 on 10
cores, ~300 concurrent claude/node processes), a fresh Claude agent
cannot finish launching within 30s, so `run create` fails with
`agent-pid-undiscoverable` and cleans up the worktree. This blocks ALL
spawn-based work (worktree-*, fan-out, orchestrate) whenever the machine
is busy — which for Jari's parallel-worktree workflow is common. Three
consecutive spawn attempts (foreground, headless, at load ~12) all failed
identically. `verify_agent_pid` in spawn.rs only range-checks the pid; the
30s window is entirely create.sh's, so raising it there is sufficient.

## Fix

Add a `--agent-startup-timeout <seconds>` flag to `run create` (validate
1–600 to match create.sh) and thread it through `SpawnRequest` →
`run_create_sh`, forwarding `cmd.arg("--agent-startup-timeout").arg(...)`.
Consider a higher default than 30s for octl specifically (e.g. 90s), since
octl spawns are frequently part of high-fan-out batches that self-load the
host. CLI-surface change: update insta snapshots + skill.rs catalog pin if
the flag appears in help output; check `orchestratectl doctor`.

## Workaround in use

Point `OCTL_CREATE_SH` at a thin wrapper that execs the real create.sh
with `--agent-startup-timeout=180` prepended. Fully reversible (unset the
env var). Used to unblock the `supervisor-dead-merge-no-teardown` bugfix
spawn on 2026-07-03.
