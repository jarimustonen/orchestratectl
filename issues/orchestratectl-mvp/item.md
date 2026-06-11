---
created: 2026-06-11
updated: 2026-06-11
type: epic
owner: jari
status: open
priority: high
---

# orchestratectl MVP

## Description

Define and ship the MVP of orchestratectl: CLI command surface, file-based state schema under `~/.orchestratectl/runs/<run-id>/`, and the minimum TUI navigation needed to begin replacing the `/worktree-*`, `/orchestrate`, and `/fan-out` skill family. Parent epic for `design.md`, `breakdown.md`, and the child issues that implement each MVP slice.

## Goal

A working, single-user, single-machine binary that:

1. Owns a canonical on-disk state schema for runs, nodes, events, discussions, and spin-off proposals.
2. Exposes a strict, AI-first CLI over that schema (`--json` everywhere, JSONL logs, no interactive prompts).
3. Spawns at least **one** worktree kind end-to-end (proposed: `spinoff`) — proving the architecture without rewriting all five `/worktree-*` variants up front.
4. Lets the human navigate runs / nodes / discussions / spin-offs from a minimum TUI without touching `tmux list-windows | grep wm-`.

MVP **explicitly excludes**: `/orchestrate` DAG runner, `/fan-out` concurrency manager, multi-host execution, and the macOS-native UI. Those land after the schema and one-kind spawn have proven stable.

## Non-goals (MVP)

- Replacing every `/worktree-*` variant. One is enough to validate.
- Backwards-compatibility with the prose skills. They run in parallel until the binary is stable.
- Cross-host or remote orchestration.
- Authentication, multi-user, or shared state.

## Design

See [`design.md`](design.md) — state schema, CLI command surface, TUI layout.

## Breakdown

See [`breakdown.md`](breakdown.md) — child issues, dependencies, critical path.

## Phases

1. **Schema + scaffolding** — `Cargo.toml`, crate layout, on-disk schema frozen in `design.md`.
2. **Read-only CLI** — `run list`, `run show`, `node list`, `node show`, `event tail`. Hand-populated fixtures verify the schema.
3. **Minimum TUI** — runs / nodes / detail panes, read-only over the same schema.
4. **First spawn** — `spinoff` kind end-to-end (creates worktree, registers node, writes events).
5. **Mutation CLIs** — `discussion resolve`, `spinoff approve|reject`.

## Notes

Built in parallel with the existing skills — both write into `~/.orchestratectl/runs/` once a thin shim from the skills lands. Until then, the binary's state is the only source of truth and skills are untouched.
