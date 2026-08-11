# orchestratectl 🎬

[![CI](https://github.com/jarimustonen/orchestratectl/actions/workflows/ci.yml/badge.svg)](https://github.com/jarimustonen/orchestratectl/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Rust CLI for orchestrating AI-agent workflows on a developer's machine.**
Spawns one or many Claude Code agents into isolated git worktrees, tracks
their progress via a file-based event log, and merges their work back —
all behind one canonical command surface.

> **Status:** v0.1.0 pre-release. Real-world ready for `/worktree-spinoff`,
> `/worktree-code` + `/worktree-merge`, and `/fan-out`. `/orchestrate` is
> battle-tested at toy scale; a handful of polish bugs gate scaled use.

## What you get

- **One autonomous agent in a worktree** — `/worktree-spinoff <task>` spawns
  an agent that works in its own git worktree and merges itself back when
  done. Zero manual cleanup.
- **One interactive agent in a worktree** — `/worktree-code <task>` for
  human-reviewed work; you merge with `/worktree-merge` when ready.
- **N identical units in parallel** — `/fan-out` spawns N≥5 disjoint
  workers with a shared manifest, automatic concurrency cap, and per-child
  cleanup.
- **A dependency-ordered campaign** — `/orchestrate` drives a DAG of
  heterogeneous features through one shared integration branch.
- **Run state on disk** — every spawn writes an event log + projections
  under `~/.orchestratectl/runs/<run-id>/`, so any UI (CLI now, TUI later)
  can read the same source of truth.
- **Agent-loop bake-off** (behind the seam) — `orchestratectl harness bakeoff
  --brief <file>` runs one coding brief through every available agent loop
  (`aider`, `claude`, `claude-deepseek`, `pi`) in isolated throwaway git repos
  and prints a side-by-side comparison (outcome / diff size / time / cost /
  checks). Not yet part of `run create`; see the code-pipeline harness (design §10).

The orchestration semantics ship as bundled Claude Code skills — install
once with `orchestratectl skill install` and the `/worktree-*`,
`/orchestrate`, `/fan-out` commands appear in any Claude Code session. A
default install **dual-homes** each skill: alongside the Claude Code copy in
`~/.claude/skills/<name>/`, it mirrors the same `SKILL.md` into
`~/.pi/agent/skills/<name>/` so the skills are also discoverable under the
[pi.dev](https://pi.dev) harness (invoked there as `/skill:<name>`). Only
`SKILL.md` is mirrored — companion resources stay Claude-only.

## Quick start

```bash
# Install from source (crates.io / homebrew coming with v0.1.0):
cargo install --path crates/octl-cli

# Deploy the bundled Claude Code skills:
orchestratectl skill install --force

# Verify:
orchestratectl doctor          # 63 ok / 0 fail expected
orchestratectl skill list      # lists 13 bundled skills

# From inside any Claude Code session in a git repo:
#   /worktree-spinoff fix the typo in src/main.rs
# orchestratectl handles spawn → work → merge → cleanup.
```

After install, the agent's first reading should be the bundled overview:

```bash
orchestratectl skill print orchestratectl-overview
```

That's the run / supervisor / node vocabulary every other skill assumes.

## How it works

Every spawn is a **run** (`~/.orchestratectl/runs/<ulid>/`). A run owns:

- `events.jsonl` — append-only event log, the canonical source of truth.
- `manifest.json`, `nodes/`, `discussions/`, `spinoffs/` — projections
  reduced from the event log under a single per-run flock.
- a **supervisor** process that owns the worktree's tmux window and
  cleans it up on terminal events.

Agents read and append events via the CLI; they never touch the projection
files directly. State is recoverable from `events.jsonl` alone, so a
crashed supervisor or a partial write never leaves a run unrunnable.

For the AI-first CLI conventions every command follows (strict input
validation, `--json` output, JSONL logs, no interactive prompts), see
[AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md).

## Bundled skills

| Skill | Purpose |
|---|---|
| `orchestratectl-overview` | First-read for the run/supervisor/node vocabulary |
| `octl-run-overview` | Inspect run state (`run list`, `run show`) |
| `octl-spawn-spinoff` | Low-level spawn primitive |
| `worktree-code` | Interactive worktree (human-reviewed merge) |
| `worktree-spinoff` | Autonomous worktree (fire-and-forget) |
| `worktree-merge` | Merge an interactive worktree back |
| `worktree-research` | Autonomous multi-source research → markdown report |
| `worktree-bugfix` | Autonomous investigate → fix → review → merge |
| `worktree-technical-decision` | Autonomous ADR-producing decision worktree |
| `worktree-make-skill` | Autonomous skill authoring with LLM review |
| `worktree-orchestrated` | Child of an `/orchestrate` DAG |
| `fan-out` | N identical units in parallel |
| `orchestrate` | Dependency-ordered DAG runner |

After installing skills, list them with `orchestratectl skill list` or
print one with `orchestratectl skill print <name>`.

## Development

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Repo layout:

- `crates/octl-core/` — schema, file I/O, locking, supervisor protocol.
- `crates/octl-cli/` — the `orchestratectl` binary and its bundled skills.
- `issues/<slug>/item.md` — every issue + epic (managed by
  [`issuectl`](https://github.com/jarimustonen/issuectl)).
- `AGENTS-AI-FIRST-CLI.md` — CLI design canon (shared with `homebase`).

## Roadmap

- v0.1.0 (in flight): zero open issues, publish to crates.io + homebrew tap.
- v0.2.0: TUI for navigating run state, resolving discussion points, and
  managing spin-off proposals.
- v0.3.0+: macOS-native UI consuming the same on-disk run state.

## License

MIT — see [LICENSE](LICENSE).
