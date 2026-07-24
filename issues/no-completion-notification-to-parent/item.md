---
created: 2026-07-24
updated: 2026-07-24
type: bug
reporter: jari
status: in-progress
priority: normal
labels: [supervisor]
commits:
- hash: b56368d
  summary: run create --notify completion hook + skill docs
- hash: ee472ee
  summary: review fixes — notify retry, reaping, env hardening
- hash: fe60e88
  summary: skill docs — supervisor env note
---

# Spawning session gets no completion notification when an async run finishes/merges

_Source: orchestration UX_

## Description

_Source: real /stint-style orchestration session in frondeo-monorepo, 2026-07-24._

## Description

When a Claude/agent conversation spawns an asynchronous run via `orchestratectl run create` (`--kind spinoff` or `--kind code`), the **spawning session receives no signal when that run later completes, merges, or fails.** `run create` returns its envelope immediately, the per-run supervisor executes out-of-band, and on `run merge` the supervisor tears the run down — but nothing is pushed back to the conversation that started it. The orchestrating agent had told the user "I'll let you know when it's done" and then never could: the user had to notice both a self-merging spinoff **and** an interactive worktree had finished and tell the agent themselves.

After teardown, `orchestratectl run show <run-id>` / `run list` return empty for the completed runs, so even a late poll yields nothing structured.

Note: the runs themselves worked perfectly — both merged cleanly to main (`fc031af`, `73023c7`). This is purely about the **missing completion signal to the spawning session**, not about run execution.

## Repro

1. From an agent conversation, `orchestratectl run create --kind spinoff …` (returns immediately).
2. Do not background a `run wait`.
3. The supervisor runs, the agent commits and calls `run merge`; the run tears down.
4. The spawning conversation is never re-invoked / notified. It only "learns" of completion if the human says so or if it happens to poll `run show` before teardown.

## Analysis / design gap

The only completion-observation primitives are **`run wait`** (blocking) and **`run show` / `event tail`** (poll). There is no push/callback to the spawning agent. In a Claude Code harness, the agent *could* launch `orchestratectl run wait <id>` as a background task so the harness re-invokes it on exit — but:

- Nothing in the `worktree-spinoff` / `worktree-code` skills instructs the agent to do this; they actively tell the agent to report completion to the user, implying a notification the tooling doesn't deliver.
- Even `run wait` only helps if the agent remembers to background it at spawn time; a fire-and-forget spinoff (the whole point) leaves no watcher.

## Suggested fixes (any one helps)

1. **`run create --notify <cmd>`** (or a config hook): on terminal state, the supervisor runs a user-supplied command — e.g. write a line to a FIFO/file the parent watches, post a desktop notification (`terminal-notifier`/`osascript`/`notify-send`), or ping the harness.
2. **Skill guidance:** when an agent spawns a run it intends to report on, have `/worktree-spinoff` and `/worktree-code` instruct it to background `orchestratectl run wait <id>` (so the harness task-notification fires), instead of promising a notification with no mechanism.
3. **Persist a terminal summary** past teardown so a late `run show <id>` still returns `{status, summary}` instead of empty — makes polling reliable.

## Acceptance criteria

- [ ] A spawning session can learn of run completion without a human relaying it — via a notify hook, documented backgrounded `run wait`, or equivalent.
- [ ] The relevant `/worktree-*` skills no longer imply a completion notification the tooling can't deliver.
- [ ] (Optional) `run show <id>` returns a terminal summary for a recently-completed run instead of empty.
