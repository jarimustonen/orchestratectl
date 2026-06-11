# orchestratectl MVP — Design

This document freezes the design decisions the MVP needs before any Rust code lands. Subsequent child issues implement against this schema; if a child issue needs to change the schema, it must update this document in the same change.

## 1. On-disk state schema

Root: `~/.orchestratectl/` (overridable via `$ORCHESTRATECTL_HOME`).

```
~/.orchestratectl/
├── runs/
│   └── <run-id>/                 # ULID, sortable, see §1.1
│       ├── manifest.json         # run-level metadata, current status
│       ├── nodes/
│       │   └── <node-id>.json    # per-agent state
│       ├── events.jsonl          # append-only event log
│       ├── discussions/
│       │   └── <discussion-id>.json
│       └── spinoffs/
│           └── <proposal-id>.json
├── index.json                    # cached run-id → summary, rebuildable
└── logs/
    └── orchestratectl.log.jsonl  # binary's own structured log
```

**Design rules:**

- One run = one directory. Deleting the directory removes the run. No central database.
- All mutable state is in JSON files or the JSONL event log. The binary holds no in-memory state between invocations.
- `events.jsonl` is the **source of truth** for transitions. `manifest.json` and `nodes/*.json` are projections rebuilt from events on read; writers update them eagerly for fast reads but must be regenerable.
- File writes use the standard create-tempfile-then-rename pattern for atomicity. `events.jsonl` is append-only with `O_APPEND` and a per-run advisory `flock` to serialize writers.
- `index.json` is a cache — if missing or stale, `orchestratectl run list` walks `runs/` and rebuilds it.

### 1.1 Identifiers

| ID | Format | Why |
|----|--------|-----|
| `run-id` | ULID (lowercase) | Sortable by creation time, unique without coordination, URL/path safe |
| `node-id` | Short slug per kind, e.g. `n-0001`, `n-0002`, monotonic within run | Human-readable when scanning the TUI |
| `discussion-id` | `d-<ULID>` | |
| `proposal-id` | `s-<ULID>` (spin-off proposal) | |

### 1.2 `manifest.json`

```json
{
  "schema_version": 1,
  "run_id": "01jx...",
  "kind": "spinoff",
  "title": "fix login redirect loop",
  "status": "running",
  "created_at": "2026-06-11T07:30:00Z",
  "updated_at": "2026-06-11T07:31:42Z",
  "source_repo": "/Users/jari/Sources/orchestratectl",
  "source_branch": "main",
  "worktree_root": "/Users/jari/Sources/orchestratectl.worktrees",
  "node_count": 1,
  "open_discussions": 0,
  "pending_spinoffs": 0
}
```

- `kind`: enum `spinoff | orchestrate | fanout | code | research | technical-decision | make-skill`. MVP populates only `spinoff`; the rest are reserved enum values so the schema does not churn when phase 5 lands.
- `status`: `pending | running | blocked | done | failed | cancelled`.

### 1.3 `nodes/<node-id>.json`

```json
{
  "schema_version": 1,
  "node_id": "n-0001",
  "run_id": "01jx...",
  "kind": "spinoff",
  "status": "running",
  "task": "investigate the redirect loop on /login",
  "worktree_path": "/Users/jari/Sources/orchestratectl.worktrees/01jx-n0001",
  "branch": "wt/01jx-n0001-login-redirect",
  "tmux_window": "wm-orchestratectl-01jx-n0001",
  "started_at": "2026-06-11T07:30:01Z",
  "updated_at": "2026-06-11T07:31:42Z",
  "last_report": null
}
```

`last_report` is `null` until the agent emits a structured report (success, discuss items, spin-off candidates) — see §1.5.

### 1.4 `events.jsonl`

One JSON object per line. Common envelope:

```json
{"ts":"2026-06-11T07:30:00.123Z","seq":42,"kind":"node.status","run_id":"01jx...","node_id":"n-0001","data":{...}}
```

`seq` is a per-run monotonic counter assigned under the write lock. `kind` namespaces the event:

- `run.created`, `run.status`
- `node.created`, `node.status`, `node.report`
- `discussion.opened`, `discussion.resolved`
- `spinoff.proposed`, `spinoff.approved`, `spinoff.rejected`

The `data` payload is event-specific. Consumers ignore unknown `kind` values for forward compatibility.

### 1.5 `discussions/<discussion-id>.json` and `spinoffs/<proposal-id>.json`

```json
{
  "schema_version": 1,
  "discussion_id": "d-01jx...",
  "run_id": "01jx...",
  "node_id": "n-0001",
  "opened_at": "2026-06-11T07:31:00Z",
  "severity": "discuss",
  "topic": "should we drop the legacy cookie path?",
  "context": "...",
  "options": ["keep", "drop", "feature-flag"],
  "status": "open",
  "resolution": null,
  "resolved_at": null
}
```

Spin-off proposals follow the same envelope with `proposed_title`, `proposed_kind`, and `accepted_as_issue_slug` once approved.

## 2. CLI command surface

Every command supports `--json` and never prompts. Errors go to stderr as `{"error":{"code":"<kebab>","message":"..."}}`. Exit codes follow `issuectl`'s convention: `0` success, `2` refused-but-actionable, `1` everything else.

### 2.1 Run

- `orchestratectl run new --kind <kind> --title <s> [--source-repo <path>] [--source-branch <s>] [--task <s>]` → prints `{run_id, dir}`.
- `orchestratectl run list [--status <s>] [--kind <s>]` → array of run summaries.
- `orchestratectl run show <run-id>` → manifest + node counts + open discussions/spinoffs.
- `orchestratectl run cancel <run-id>` → marks status `cancelled`, emits event.

### 2.2 Node

- `orchestratectl node list <run-id>`
- `orchestratectl node show <run-id> <node-id>`
- `orchestratectl node report <run-id> <node-id> --from-file report.json` — used by the agent (or shim) to attach its structured report.

### 2.3 Event

- `orchestratectl event tail <run-id> [--from-seq <n>] [--follow]` — JSONL stream. `--follow` keeps the file open.

### 2.4 Discussion

- `orchestratectl discussion list <run-id> [--status open|resolved]`
- `orchestratectl discussion show <run-id> <discussion-id>`
- `orchestratectl discussion resolve <run-id> <discussion-id> --choice <s> [--note <s>]`

### 2.5 Spin-off

- `orchestratectl spinoff list <run-id>`
- `orchestratectl spinoff approve <run-id> <proposal-id> [--issue-slug <s>]`
- `orchestratectl spinoff reject <run-id> <proposal-id> [--reason <s>]`

### 2.6 TUI

- `orchestratectl tui` — launches the terminal UI; same binary, no subcommand args beyond optional `--run <run-id>` to start drilled in.

## 3. Minimum TUI

Three-pane Miller-column layout (matches `gog`/Finder mental model the user already uses):

```
┌─ Runs ───────────────┬─ Nodes ──────────────┬─ Detail ──────────────┐
│ > 01jx… spinoff      │ > n-0001 running     │ Task: ...             │
│   01jx… orchestrate  │   n-0002 done        │ Branch: ...           │
│   ...                │                       │ Last report: ...      │
└──────────────────────┴──────────────────────┴───────────────────────┘
 [j/k] move  [h/l] pane  [enter] drill  [d] discussions  [s] spinoffs  [q] quit
```

- Read-only in MVP. Mutations (`discussion resolve`, `spinoff approve`) come via the CLI; the TUI just renders state.
- Refresh on a 500ms tick by re-reading the open run's manifest + events from `--from-seq <last_seen>`. No file watchers in MVP — polling is good enough single-user.
- Status footer shows totals (running / blocked / done / open discussions / pending spin-offs) for the focused run.

## 4. Concurrency and safety

- Per-run advisory `flock` on `<run-dir>/.lock` for any writer.
- Reads do not lock; they tolerate a torn `manifest.json` (atomic rename means a reader either sees the old or new file, never a partial).
- `events.jsonl` is append-only; the `seq` counter is read from the last line under the lock, then incremented.
- The TUI does not write to the schema — it only reads.

## 5. Crate layout

```
orchestratectl/
├── Cargo.toml
└── crates/
    ├── octl-core/        # schema types, file I/O, locking
    ├── octl-cli/         # clap-based CLI, --json plumbing
    └── octl-tui/         # ratatui frontend
```

Top-level `Cargo.toml` is a workspace; `cargo install --path crates/octl-cli` produces the `orchestratectl` binary which links `octl-tui` for the `tui` subcommand. This keeps the schema crate dependency-light and reusable from future host UIs.

## 6. Dependency picks (MVP)

| Concern | Pick | Rationale |
|---|---|---|
| CLI parsing | `clap` v4 with derive | Standard, AI-first-friendly help output |
| Serialization | `serde` + `serde_json` | Schema is JSON |
| ID generation | `ulid` | Sortable, no coordination |
| Filesystem locking | `fs2` | Cross-platform `flock` |
| Errors | `anyhow` (CLI), `thiserror` (core) | Library/binary split |
| TUI | `ratatui` + `crossterm` | Currently dominant; active maintenance |
| Logging | `tracing` + `tracing-subscriber` JSON layer | JSONL log output requirement |
| Tests | `insta` for snapshot, `tempfile` for fixtures | Schema is easy to snapshot |

## 7. Open questions (resolve before phase 5)

- How does the binary discover existing tmux windows for nodes that pre-date it? — Likely a one-time `import` command; out of MVP scope.
- Do we want a `--watch` mode on `run show` for parity with `tui`, or is `event tail --follow` enough? — Decide once `event tail` ships.
- Schema migration story (`schema_version: 1 → 2`). — Defer; we will need it but not for MVP.
