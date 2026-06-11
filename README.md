# orchestratectl 🎬

Rust CLI + TUI for orchestrating AI-agent workflows on a developer's
machine. Bundles the orchestration semantics behind the `/worktree-*`,
`/orchestrate`, `/fan-out`, and `/llm-*` Claude Code skills into one
canonical command surface, with a terminal UI for navigating run status,
resolving discussion points, and managing spin-off issues.

**Status:** Private, early. MVP in progress.

## What it does

- **Spawn** worktree agents (`code`, `spinoff`, `orchestrated`, `research`,
  `technical-decision`) under one consistent flag surface
- **Orchestrate** heterogeneous, dependency-ordered campaigns of agents
  (DAG runner) and **fan out** identical units across many parallel workers
- **Persist run state** to `~/.orchestratectl/runs/<run-id>/` so multiple
  UIs can read the same source of truth
- **TUI** for browsing runs, drill-down into worktrees, reviewing
  `/worktree-status` output, resolving discussion points, approving or
  rejecting spin-off proposals
- **Workmux integration** to track which tmux window owns which worktree

## Why

The current skill family is large and the orchestration logic lives in
prose inside `SKILL.md` files. `orchestratectl` extracts that logic into
tested code, exposes it via an AI-first CLI, and gives the human a
keyboard-driven TUI to navigate the resulting state instead of `tmux
list-windows | grep wm-`.

## License

MIT (forthcoming).
