---
created: 2026-06-11
updated: 2026-06-13
type: epic
owner: jari
status: done
priority: high
closed: 2026-06-13
---

# orchestratectl MVP

## Description

Define and ship the MVP of orchestratectl: CLI command surface, file-based state schema under `~/.orchestratectl/runs/<run-id>/`, and the minimum TUI navigation needed to begin replacing the `/worktree-*`, `/orchestrate`, and `/fan-out` skill family. Parent epic for `design.md`, `breakdown.md`, and the child issues that implement each MVP slice.

## Goal

A working, single-user, single-machine binary that:

1. Owns a canonical on-disk state schema for runs, nodes, events, discussions, and spin-off proposals.
2. Exposes a strict, AI-first CLI over that schema (`--json` everywhere, JSONL logs, no interactive prompts).
3. Spawns **all 8 current worktree kinds** end-to-end at the single-agent level — `code`, `spinoff`, `orchestrated`, `research`, `technical-decision`, `make-skill`, `fan-out`, `bugfix`. The recursive supervisor design (see `design.md` §7) makes "all kinds" only marginally more code than "one kind" because the spawn path delegates to the existing `~/.claude/skills/worktree/scripts/create.sh` for window naming, emoji, and worktree creation.
4. Reliably surfaces structured decision reports (`node.report` events) from terminated agents to their parent supervisor — exactly-once consumption — so post-MVP DAG/fan-out work can build on the same protocol.

MVP **explicitly excludes**: a native Rust DAG runner, a native Rust fan-out concurrency manager, multi-host execution, the macOS-native UI, and the read-only TUI. The TUI is deferred to a later phase; the human navigates runs via CLI + `event tail --follow` for MVP. DAG and fan-out concurrency stay in the respective agent skill prompts, which call the binary recursively (parent agent → `orchestratectl run create --kind ...` per child).

## Non-goals (MVP)

- Native Rust orchestration logic (DAG runner, fan-out concurrency manager). These live in agent skill prompts and use the binary as primitive.
- Backwards-compatibility with the prose skills' internal state. They run in parallel until the binary is stable; both write into `~/.orchestratectl/runs/` once the shim lands.
- Cross-host or remote orchestration.
- Authentication, multi-user, or shared state.
- TUI. Read-only TUI is deferred; CLI + `event tail --follow` is the MVP human view.

## Design

See [`design.md`](design.md) — state schema, CLI command surface, TUI layout.

## Breakdown

See [`breakdown.md`](breakdown.md) — child issues, dependencies, critical path.

## Phases

1. **Schema + scaffolding** — `Cargo.toml`, two-crate workspace (`octl-core`, `octl-cli`), on-disk schema frozen in `design.md`.
2. **Read-only CLI** — `run list`, `run show`, `node list`, `node show`, `event tail`. Hand-populated fixtures verify the schema.
3. **Mutation CLIs** — `discussion resolve`, `spinoff approve|reject`, `run cancel`, `node report`.
4. **Supervisor + all-kinds spawn** — recursive per-spawning-agent supervisor process, `run create --kind <X>` shells out to `create.sh` and registers the node, watchdog handles `node.report` and child-process death.

## Issues

Child issues (see `breakdown.md` for full dependencies and critical path):

- [ ] @cargo-scaffolding — Workspace + AI-first CLI plumbing (chore)
- [ ] @state-schema-crate — octl-core schema types + flock primitive
- [ ] @run-cli-read — `run create|list|show|cancel|reattach`
- [ ] @node-cli-read — `node list|show|report`
- [ ] @event-tail-cli — `event tail` with `--follow`, SIGINT/SIGTERM
- [ ] @event-create-cli — sanctioned write path for skill-shim
- [ ] @version-subcommand — `version --json` per AGENTS-AI-FIRST-CLI §10
- [ ] @skill-subcommand — companion-skill installer per §15
- [ ] @supervisor-process — `orchestratectl supervise` long-lived
- [ ] @all-kinds-spawn — `run create --kind <X>` for all 8 kinds
- [ ] @discussion-cli — `discussion list|show|resolve`
- [ ] @spinoff-proposal-cli — `spinoff list|approve|reject`
- [ ] @create-sh-structured-stdout — cross-repo patch to `create.sh` (chore)

## Comments

Built in parallel with the existing skills — both write into `~/.orchestratectl/runs/` once a thin shim from the skills lands. Until then, the binary's state is the only source of truth and skills are untouched.
