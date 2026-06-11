# orchestratectl MVP — Breakdown

Child issues that deliver the epic. Each is sized to ~one session of focused work. Dependencies drawn from `design.md`.

## Critical path

```
cargo-scaffolding
        │
        ▼
state-schema-crate ─────┬─────────────┐
        │               │             │
        ▼               ▼             ▼
run-cli-read     event-tail-cli   node-cli-read
        │               │             │
        └───────┬───────┘             │
                ▼                     │
        tui-minimum-navigation ◄──────┘
                │
                ▼
        spinoff-spawn ──► discussion-cli ──► spinoff-proposal-cli
```

The critical path runs through `cargo-scaffolding` → `state-schema-crate` → `run-cli-read` → `tui-minimum-navigation` → `spinoff-spawn`. Everything else is parallelizable once `state-schema-crate` is in.

## Child issues

| # | Slug | Type | Depends on | Summary |
|---|------|------|------------|---------|
| 1 | `cargo-scaffolding` | chore | — | Workspace `Cargo.toml`, three crates (`octl-core`, `octl-cli`, `octl-tui`), CI-friendly defaults, AI-first CLI plumbing (`--json`, JSONL log subscriber, stderr error envelope). |
| 2 | `state-schema-crate` | feature | 1 | `octl-core`: schema types (manifest, node, event, discussion, spinoff), atomic write helpers, per-run flock, event append + seq counter. Snapshot tests against fixture runs. |
| 3 | `run-cli-read` | feature | 2 | `orchestratectl run new\|list\|show\|cancel`. `new` initializes a run dir; reads are pure projection over schema. |
| 4 | `node-cli-read` | feature | 2 | `orchestratectl node list\|show\|report`. `report` ingests a JSON file and emits `node.report` event. |
| 5 | `event-tail-cli` | feature | 2 | `orchestratectl event tail` with `--from-seq` and `--follow` (poll-based tail; no inotify in MVP). |
| 6 | `tui-minimum-navigation` | feature | 3, 4 | Three-pane ratatui UI, read-only, 500ms refresh polling the focused run. j/k/h/l/enter/q + d/s shortcuts to lists. |
| 7 | `spinoff-spawn` | feature | 3 | `orchestratectl run new --kind spinoff` actually creates the worktree, branch, tmux window, and registers `n-0001`. First end-to-end agent kind. |
| 8 | `discussion-cli` | feature | 2 | `orchestratectl discussion list\|show\|resolve` (mutation: writes `discussion.resolved` event, updates JSON). |
| 9 | `spinoff-proposal-cli` | feature | 2 | `orchestratectl spinoff list\|approve\|reject`. `approve` optionally calls `issuectl new` to materialize an issue. |

## Out of scope for MVP (tracked for later)

- `orchestrate-dag-runner` — full DAG executor.
- `fanout-runner` — parallel identical units, manifest-tracked resume.
- `skills-shim` — make `/worktree-*` skills write into `~/.orchestratectl/runs/` so old and new coexist.
- `import-existing-tmux` — adopt pre-existing wm-* tmux windows into a synthetic run.
- `schema-migration-v2` — once schema_version changes.
- `macos-native-ui` — host UI consuming the same schema.

## Sequencing notes

- Issues 4 and 5 can run in parallel once 2 lands.
- Issue 7 (`spinoff-spawn`) is the riskiest — it touches `git worktree`, `tmux`, and process spawning. Worth a `validation.md` before coding to confirm the exact `git`/`tmux` commands and failure modes.
- Issues 8 and 9 are mostly schema + CLI plumbing; cheap once 2 and 3 are in.
- The TUI (6) is parallelizable with 7/8/9 since it is read-only.

## Resolution of design open questions

The three open questions in `design.md` §7 are deferred — they do not block any MVP child issue. They become inputs to the post-MVP planning round.
