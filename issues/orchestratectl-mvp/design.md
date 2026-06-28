# orchestratectl MVP — Design

This document freezes the design decisions the MVP needs before any Rust code lands. Subsequent child issues implement against this schema; if a child issue needs to change the schema, it must update this document in the same change.

The decisions in this document were locked through a multi-LLM design workshop (Fork-by-Fork ideation across 4 frontier models, then user dialogue). See [`alternatives.md`](alternatives.md) for the alternatives that were seriously considered and rejected, and [`validation.md`](validation.md) for the assumptions that remain to be empirically validated.

## 0. Locked decisions at a glance

| Fork | Decision | Why (one line) |
|---|---|---|
| **Fork 1** — State persistence | JSONL `events.jsonl` source-of-truth + JSON projections, per-run advisory `flock` | Skill-shim can `cat`/`tail`/edit, replay-clean idiomatically, single-writer atomicity comes from POSIX |
| **Fork 2** — Process model | Recursive per-spawning-agent supervisor; short-lived CLI for everything else | Gives `node.report` exactly-once consumption a home; matches the recursive spawn model agents already need |
| **Fork 3** — CLI surface | Linear/git-style `<noun> <verb>` subcommands, strict AI-first conventions | Predictable surface; binds naturally to AGENTS-AI-FIRST-CLI.md |
| **Fork 4** — TUI | Out of MVP. Deferred to a post-MVP phase. | Removes the largest scope risk; CLI + `event tail --follow` is enough for MVP |
| **Fork 5** — Worktree+tmux spawning | Shell-out to `~/.claude/skills/worktree/scripts/create.sh` in MVP; native Rust port post-MVP | Zero divergence from skill family during coexistence; minimal MVP code |

Each Decision/Why/Trade-offs block below expands one of these rows.

## 1. On-disk state schema (Fork 1)

**Decision:** Per-run JSON projections + append-only `events.jsonl` source-of-truth, serialized by a per-run advisory `flock`.

**Why:** The event log is replayable in principle without any extra tooling — `cat events.jsonl | jq` and a human-readable replay are the same operation. JSON projections give CLI reads fast non-parsing scans. Concurrent writers are serialized by the kernel `flock`; readers do not lock and tolerate "may not see the last in-flight write yet" semantics.

**Sanctioned write path for non-Rust callers (skill-shim):** **only `orchestratectl event create`** is sanctioned. Direct `echo ... >> events.jsonl` (with or without `flock(1)`) is **explicitly banned** because macOS does not ship `flock(1)` natively and a portable shell-side locking discipline cannot be enforced. The binary handles `flock`, `seq` assignment, projection updates, and fsync in one atomic step.

**Trade-offs accepted:**
- Writes are amplified: append event + atomically rewrite each touched projection file. At ~100 concurrent agents and ~10 events/sec each, this is ~1000 small writes/sec — well within laptop FS capacity but not free.
- Cross-file consistency is not guaranteed across a crash: an event may land before its projection rewrite. The event log remains canonical, so a future `rebuild-projections` tool can heal projections. MVP does not ship that tool; in MVP the discipline is "projections are best-effort caches, event log is truth, crash recovery is treated as a known gap until v2."
- Ad-hoc cross-run queries ("show every open discussion across runs") are a directory walk + JSON parse — fine at MVP scale, would degrade at 10× scale.
- Advisory `flock` is cooperative: a non-conforming script that writes the file directly without locking can still corrupt state. Mitigation: skill-shim documentation explicitly bans direct file writes; `orchestratectl event create` is the only sanctioned path.
- `seq` counter must be re-read from the last line of `events.jsonl` for every short-lived `event create`; for long-running supervisors the counter is cached in memory. Watched performance risk for very long logs (see `validation.md`).

Root: `~/.orchestratectl/` (overridable via `$ORCHESTRATECTL_HOME`).

```
~/.orchestratectl/
├── runs/
│   └── <run-id>/                 # ULID, sortable, see §1.1
│       ├── manifest.json         # run-level metadata, current status
│       ├── nodes/
│       │   └── <node-id>.json    # per-agent state
│       ├── events.jsonl          # append-only event log (canonical)
│       ├── discussions/
│       │   └── <discussion-id>.json
│       ├── spinoffs/
│       │   └── <proposal-id>.json
│       ├── supervisor.pid        # per-run-root supervisor PID (when alive)
│       └── .lock                 # per-run advisory flock target
├── index.json                    # cached run-id → summary, rebuildable
└── logs/
    └── orchestratectl.log.jsonl  # binary's own structured log
```

**Design rules:**

- One run = one directory. Deleting the directory removes the run. No central database.
- All mutable state is in JSON files or the JSONL event log. The binary holds no in-memory state between invocations except inside long-lived supervisor processes (see §7).
- `events.jsonl` is the **source of truth** for transitions. `manifest.json` and `nodes/*.json` are projections rebuilt from events on read; writers update them eagerly for fast reads but they must be regenerable in principle.
- File writes use the standard create-tempfile-then-rename pattern for atomicity. `events.jsonl` is append-only with `O_APPEND` under per-run `flock`.
- `index.json` is a cache — if missing or stale, `orchestratectl run list` walks `runs/` and rebuilds it.

### 1.1 Identifiers

| ID | Format | Why |
|----|--------|-----|
| `run-id` | ULID (lowercase) | Sortable by creation time, unique without coordination, URL/path safe |
| `node-id` | Short slug per kind, e.g. `n-0001`, `n-0002`, monotonic within run | Human-readable when scanning event tail or logs |
| `discussion-id` | `d-<ULID>` | |
| `proposal-id` | `s-<ULID>` (spin-off proposal) | |

### 1.2 `manifest.json`

```json
{
  "schema_version": 1,
  "run_id": "01jx...",
  "kind": "spinoff",
  "lifecycle": "autonomous",
  "title": "fix login redirect loop",
  "status": "running",
  "created_at": "2026-06-11T07:30:00Z",
  "updated_at": "2026-06-11T07:31:42Z",
  "source_repo": "/Users/jari/Sources/orchestratectl",
  "source_branch": "main",
  "worktree_root": "/Users/jari/Sources/orchestratectl.worktrees",
  "node_count": 1,
  "open_discussions": 0,
  "pending_spinoffs": 0,
  "parent_run_id": null,
  "parent_node_id": null
}
```

- `kind`: enum, all 8 kinds active in MVP — `code | spinoff | orchestrated | research | technical-decision | make-skill | fan-out | bugfix`.
- `lifecycle`: enum `autonomous | interactive`.
  - `autonomous` runs (spinoff, research, orchestrated, fan-out, bugfix, technical-decision, make-skill) terminate themselves on completion and supervisor watchdog treats unexpected exit as `failed`.
  - `interactive` runs (code) wait for human interaction; agent may self-terminate but typically requires explicit `orchestratectl run cancel` or a `/worktree-merge`-style closure. Watchdog tolerates the agent process living indefinitely.
  - Caller-side semantics are identical (both produce a final `node.report` event when they actually finish).
- `status`: `pending | running | blocked | done | failed | cancelled`.
- `parent_run_id` / `parent_node_id`: when a child run is spawned by a parent agent via `orchestratectl run create` from inside another run, these fields record the spawning context. This is what makes the recursive supervisor model coherent — every run knows its parent.

### 1.3 `nodes/<node-id>.json`

```json
{
  "schema_version": 1,
  "node_id": "n-0001",
  "run_id": "01jx...",
  "parent_node_id": null,
  "kind": "spinoff",
  "status": "running",
  "task": "investigate the redirect loop on /login",
  "worktree_path": "/Users/jari/Sources/orchestratectl.worktrees/01jx-n0001",
  "branch": "wt/01jx-n0001-login-redirect",
  "tmux_window": "🚀 wt/01jx-n0001-login-redirect",
  "tmux_identity": {
    "socket": "/private/tmp/tmux-501/default",
    "session": "orchestratectl",
    "window_id": "@42"
  },
  "agent_pid": 47821,
  "agent_pid_start_time": "2026-06-11T07:30:01.420Z",
  "supervisor_pid": 47820,
  "children": [],
  "started_at": "2026-06-11T07:30:01Z",
  "updated_at": "2026-06-11T07:31:42Z",
  "last_report": null,
  "last_processed_report_seq_by_child": {}
}
```

- `parent_node_id`: the spawning agent's node-id when a parent agent spawned this child. `null` for run roots.
- `tmux_window`: `<emoji> <branch-name>` per the convention in `~/.claude/skills/worktree/scripts/create.sh` (e.g., `🚀` for spinoff, `💻` for code, `🔬` for research, `🪭` for fan-out). Human-readable name only — not unique across sessions; prefer `tmux_identity` for liveness.
- `tmux_identity`: fully-qualified `{socket, session, window_id}` captured at spawn (see §8.1) so the watchdog matches the exact window instead of a bare name. `null` for nodes registered before `create.sh` emitted the qualified fields (or that emitted a partial/empty identity) — those fall back to bare-name matching on `tmux_window`. `window_id` is the stable `@NNNN` form (unique per server, survives renames) and is what the watchdog matches; `session` is display-only; `socket` is normally the resolved `#{socket_path}` and is `null` only when create.sh could not read it.
- `agent_pid`: the agent process PID extracted from `create.sh`'s structured output (see §8). Liveness probed via `kill(agent_pid, 0)` polling (see §7.5).
- `agent_pid_start_time`: process start timestamp captured at spawn. Used together with `agent_pid` to defeat PID reuse — after a long sleep or reboot, the supervisor verifies both PID and start-time match before treating the agent as the same process. On macOS via `kinfo_proc.kp_proc.p_starttime`; on Linux via `/proc/<pid>/stat` field 22.
- `supervisor_pid`: this run's supervisor PID. Stored separately in `<run-dir>/supervisor.pid` as well for cross-run discovery.
- `children`: array of `{run_id, node_id}` for child runs spawned by *this node's* agent. Populated when this node calls `orchestratectl run create --parent-run-id ... --parent-node-id <this>`. Critical for parent-supervisor restart recovery (§7.6).
- `last_report`: `null` until the agent emits a structured report (success, discuss items, spin-off candidates) — see §1.5 and §7.
- `last_processed_report_seq_by_child`: map of `{child_run_id: seq}` recording which child reports this supervisor has already consumed. Used for exactly-once consumption across restarts (§7.3).

### 1.4 `events.jsonl`

One JSON object per line. Common envelope:

```json
{"ts":"2026-06-11T07:30:00.123Z","seq":42,"kind":"node.status","run_id":"01jx...","node_id":"n-0001","data":{...}}
```

`seq` is a per-run monotonic counter assigned under the write lock. `kind` namespaces the event:

- `run.created`, `run.status`
- `node.created`, `node.status`, `node.report`, `node.heartbeat` (optional, see §7.5)
- `child.spawned` — written by short-lived `run create` CLI to the **parent run's** event log when a child run is created; this is what makes the parent supervisor able to discover its children via tail-follow (§7.2)
- `discussion.opened`, `discussion.resolved`
- `spinoff.proposed`, `spinoff.approved`, `spinoff.rejected`
- `supervisor.started`, `supervisor.exited` — tracks per-run supervisor lifecycle for debuggability
- `orchestrator.decision`, `discuss.critical` — append-only audit records from `/orchestrate` (its decision log and pakkopysäytys). They carry no projection (the reducer folds them to a no-op); the event log is their canonical home. Not `node.report`, so the supervisor's terminal roll-up ignores them.

The `data` payload is event-specific. Consumers ignore unknown `kind` values for forward compatibility.

`node.report` payloads can be **large** (10–50 KB is realistic for a research-kind agent reporting its full conclusions, or an orchestrated-kind agent enumerating 12 sub-agent outcomes with notes). JSONL handles arbitrarily long lines; the write happens under `flock` so other writers don't interleave.

**Deterministic IDs for derived events.** When a parent supervisor consumes a child's `node.report` and emits downstream events (`discussion.opened`, `spinoff.proposed`), the resulting `discussion_id` / `proposal_id` are **deterministic hashes** of the source tuple:

```
discussion_id = "d-" + base32(sha256(child_run_id || ":" || child_node_id || ":" || report_seq || ":" || item_index)[:10])
proposal_id   = "s-" + base32(sha256(child_run_id || ":" || child_node_id || ":" || report_seq || ":" || item_index)[:10])
```

This makes restart-recovery automatically de-duplicate: if the parent supervisor crashes between emitting the consumption events and recording `last_processed_report_seq_by_child`, the restart sees the same `report_seq` + `item_index`, computes the same IDs, and the projection-write step **skips** writing duplicate JSON files (because `discussions/<id>.json` already exists with identical content). The event-log append also detects the duplicate by ID and skips. Replay is deterministic and idempotent.

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

Spin-off proposals follow the same envelope with `proposed_title`, `proposed_kind`, and `accepted_as_issue_slug` once approved. Approval may auto-create an `issuectl` issue.

## 2. CLI command surface (Fork 3)

**Decision:** Resource-oriented `orchestratectl <noun> <verb>` subcommand tree, modeled after the Linear CLI and git's verb-centric ergonomics. Strict AI-first conventions per [`AGENTS-AI-FIRST-CLI.md`](../../AGENTS-AI-FIRST-CLI.md) bind everywhere.

**Why:** Noun-verb is predictable and uniform — every noun supports the same verb vocabulary (`list`, `show`, `create`, `update`, `delete`) wherever the action makes sense, so the AI caller can guess valid commands. Linear's surface is one Jari already likes; git's lets short root-noun-bare verbs exist when the entity is frequent (e.g., `event tail` like `git log`). AI-first conventions make every output parseable and every error actionable.

**Trade-offs accepted:**
- Verbose for humans: `orchestratectl run create --kind spinoff --title "..."` is more typing than a manifest-apply model (`octl apply run.yaml`). Manifest mode is deferred to post-MVP; AGENTS-AI-FIRST-CLI §6 explicitly limits `apply` to cases where convergent reconciliation is real semantics, not aesthetic.
- Strict input rejects empty/whitespace/unknown flags. Skills that pass through user input must validate first; agents that build commands programmatically must construct them well.

### 2.0 Binding conventions (apply to every subcommand)

These are AGENTS-AI-FIRST-CLI bindings for the whole CLI surface:

**Verb vocabulary** (§7): `list`, `show`, `create`, `update`, `delete` are the only allowed CRUD verbs (no synonyms — never `new`, `add`, `get`, `rm`, etc.). Domain verbs are written-justified exceptions:
- `cancel` (run) — terminal state transition with active side effects (signal supervisor), not a soft `update --status=cancelled`. Like `git revert`.
- `resolve` (discussion) — closing a deliberation is a domain action with `--choice` + `--note`, not a generic field mutation.
- `approve` / `reject` (spinoff) — both terminal decisions; modeling as `update --status=approved` would obscure the materialization side effect (calling `issuectl new`).
- `report` (node) — agent self-submission of a structured terminal payload, semantically distinct from a generic node update. The payload follows a fixed schema (§7.3).
- `tail` (event) — long-running streaming verb. Like `git log --follow`.

**Schema versioning** (§10): every `--json` payload (top-level and event-level) carries `schema_version` (integer). MVP starts at `1`. The state schema's own `schema_version` (in `manifest.json`, etc.) is separate from the CLI output's `schema_version`.

**Error envelope** (§10): failures emit to **stderr** as:

```json
{
  "schema_version": 1,
  "error": {
    "code": "<snake_case>",
    "message": "<human-readable>",
    "invalid_value": "<the bad input, if applicable>",
    "expected": ["<allowed values, if enumerable>"]
  }
}
```

**Warnings** (§10): non-fatal warnings live in `warnings: []` on the stdout payload (under `--json`), not on stderr. Stderr stays fatal-only.

**Dry-run** (§11): every mutating command (`run create`, `run cancel`, `discussion resolve`, `spinoff approve`/`reject`, `node report`, `event create`) supports `--dry-run` and emits a planning envelope without applying. Truly server-side-generated state (e.g., assigned `run_id`) is enumerated under `unknown_until_apply`.

**Idempotency** (§11): `run create`, `node report`, and `event create` all accept `--idempotency-key <opaque>`. A second call with the same key returns the original result instead of creating a duplicate event/resource. Critical for the recursive spawn model: when an agent retries `orchestratectl run create` (after a network blip, timeout, or its own crash + restart), it must not create a duplicate child run. The key is stored alongside the resource (in `manifest.json` for runs, in the event's `idempotency_key` field for events); a repeat call with the same key short-circuits to "return the prior result." When omitted, calls are non-idempotent and behave per the standard verb semantics.

**Output format** (§9): format determined only by `--format=<text|json|jsonl>` flag (`--json` is shorthand for `--format=json`). No TTY detection. No color/spinner/progress bars. No automatic pagination.

**Large outputs** (§13): when a list might be large, **`--output FILE.jsonl`** writes records to a file and stdout returns metadata `{path, count, schema_version}`. The `--format` flag controls the format inside the file as well. `--format` and `--output` are independent flags — never overloaded onto the same name.

**Streaming terminal events** (§12): every long-running streaming command emits exactly one terminal event before exit:
- Natural completion: `{"event":"result","schema_version":1, ...}`
- User cancellation (SIGINT/SIGTERM): `{"event":"cancelled","schema_version":1}`
- Crash: no terminal event (consumer treats absence as `error`)

**Help** (§14): `<subcommand> --help` includes flag list + accepted values + env-var mapping + an `examples: []` array. `<subcommand> --help --json` returns structured help.

**Signal handling** (§12): long-running streaming commands (`event tail --follow`, `orchestratectl supervise`) trap `SIGINT` and `SIGTERM`. SIGINT exits 130; SIGTERM exits 143. A final `{"event":"cancelled","schema_version":1}` event is emitted to stdout before exit when feasible.

### 2.1 Run

- `orchestratectl run create --kind <kind> --title <s> [--source-repo <path>] [--source-branch <s>] [--task <s>] [--parent-run-id <id>] [--parent-node-id <id>] [--idempotency-key <opaque>] [--dry-run]` → prints `{schema_version, run_id, dir, supervisor_pid}` on success. `--kind` accepts any of the 8 active kinds; unknown rejected with structured error. Behavior diverges based on whether this is a **top-level run** or a **child spawn**:
  - **Top-level (no `--parent-*` flags):** the CLI initializes the run directory, writes `run.created` + `node.created` to the new run's events, spawns the supervisor (`orchestratectl supervise <run-id>`), and exits. Supervisor takes over from there.
  - **Child spawn (`--parent-run-id` + `--parent-node-id` set):** the CLI writes the `child.spawned` event to the **parent run's** events.jsonl (under parent's `flock`), creates the new child run dir, writes `run.created` + `node.created` to the child run's events, and **exits without spawning a supervisor**. The parent supervisor sees the `child.spawned` event in its tail-follow loop and spawns the child supervisor itself. This makes the parent the single source of authority for "who owns this child."
  - **`--dry-run`**: for top-level creation `--dry-run` plans the run-dir scaffolding and supervisor spawn — but because spawning a real supervisor process is a non-dryable side effect, the planning envelope marks `supervisor_pid`, `tmux_window`, and `agent_pid` as `unknown_until_apply`. For child spawn, `--dry-run` is restricted to validating arguments and parent existence; if a truthful dry-run is not possible (e.g., the parent supervisor must observe the event to act), exits with `dry_run_unsupported` per AGENTS-AI-FIRST-CLI §11. Truthful idempotent retry via `--idempotency-key` is the safer alternative.
- `orchestratectl run list [--status <s>] [--kind <s>]`
- `orchestratectl run show <run-id>`
- `orchestratectl run cancel <run-id> [--dry-run]` → marks status `cancelled`, emits `run.status: cancelled` event, **synthesizes a terminal `node.report` event** for each non-terminal node in the run with `{success: false, reason: "cancelled by user", cancelled: true}` so parent supervisors waiting on a terminal report do not hang. Signals the supervisor process to exit gracefully. Domain verb (§2.0).
- `orchestratectl run reattach <run-id> [--dry-run]` → restarts the supervisor for an existing run whose `supervisor.pid` is stale or whose process has exited. Reads the run's node JSONs (especially `children`) to rebuild the in-memory tail-follow loops for any in-flight children. Replays unprocessed `node.report` events from children whose `last_processed_report_seq_by_child` lags. Domain verb (§2.0).

### 2.2 Node

- `orchestratectl node list <run-id>`
- `orchestratectl node show <run-id> <node-id>`
- `orchestratectl node report <run-id> <node-id> --from-file report.json [--idempotency-key <opaque>] [--dry-run]` — agent self-submission of structured terminal report (domain verb, §2.0). Payload schema in §7.3. `--idempotency-key` defends against agent retries (two `node report` calls with the same key produce one `node.report` event). When omitted, no deduplication and a second call would create a second report event.

### 2.3 Event

- `orchestratectl event tail <run-id> [--from-seq <n>] [--follow] [--format=text|json|jsonl] [--output <FILE>]` — streams run events.
  - **Without `--follow`**: streams from `--from-seq` (default 0) to EOF and emits a final `{"event":"result","schema_version":1,"total":<N>}` terminal record before exit.
  - **With `--follow`**: polls the file (no inotify in MVP) for new events; traps SIGINT (exit 130) / SIGTERM (exit 143) and emits `{"event":"cancelled","schema_version":1}` before exit.
  - Each event line carries `schema_version`, `seq`, and the canonical event shape from §1.4.
  - `--format=jsonl` is the default when streaming; `--format=text` is human-oriented; `--format=json` rejects streaming mode (use `--output FILE.json` for batch capture if you need a single document).
  - `--output FILE` writes records to the file instead of stdout; stdout returns `{path, count, schema_version}` metadata.
- `orchestratectl event create <run-id> --kind <k> --node-id <n> --from-file <data.json> [--idempotency-key <opaque>] [--dry-run]` — sanctioned write path for skill-shim and external tools that don't want to manage `flock` themselves. The binary acquires the run's `flock`, assigns `seq`, appends the event, **runs the reducer to update the affected projection files atomically (e.g., a `discussion.opened` event updates `discussions/<id>.json` and `manifest.json.open_discussions`)**, fsyncs, releases the lock. Without the reducer step, sanctioned-write callers would produce drift between the canonical event log and the projection files that read CLIs depend on. Validates `kind` against the known event-kind set; unknown rejected.

### 2.4 Discussion

- `orchestratectl discussion list <run-id> [--status open|resolved]`
- `orchestratectl discussion show <run-id> <discussion-id>`
- `orchestratectl discussion resolve <run-id> <discussion-id> --choice <s> [--note <s>] [--dry-run]` — domain verb (§2.0).

### 2.5 Spin-off

- `orchestratectl spinoff list <run-id>`
- `orchestratectl spinoff approve <run-id> <proposal-id> [--issue-slug <s>] [--dry-run]` — domain verb (§2.0).
- `orchestratectl spinoff reject <run-id> <proposal-id> [--reason <s>] [--dry-run]` — domain verb (§2.0).

### 2.6 Version

- `orchestratectl version [--json]` → returns `{schema_version, version, commit, state_schema_version, supported_state_schemas: [1]}`. The `state_schema_version` is the writable schema; `supported_state_schemas` lists schemas this binary can read.

### 2.7 Skill (companion-skill installer)

Per AGENTS-AI-FIRST-CLI §15, orchestratectl ships its own AI-operating-manual skills alongside the binary. Skill files live in-repo under `crates/octl-cli/skills/` and ship with the binary.

- `orchestratectl skill list [--json]` → lists shipped skills with one-line descriptions.
- `orchestratectl skill show <name> [--json]` → prints the skill content without installing.
- `orchestratectl skill install [<name>] [--target <dir>]` → copies the skill(s) into `~/.claude/skills/` (or `--target`). Installs all when no name given.

MVP scope ships the **subcommand and mechanics**, with a minimal seed skill set (`octl-run-overview`, `octl-spawn-spinoff`) that proves the path. The full skill library (replacing `/worktree-*` family) lands as a follow-up once the CLI is stable and the surface won't churn under the skill texts.

### 2.8 No TUI subcommand in MVP

The `orchestratectl tui` subcommand from the first-pass draft is removed from MVP scope. See §3.

## 3. TUI — deferred (Fork 4)

**Decision:** No TUI in MVP. The human navigates runs via CLI and `event tail --follow`.

**Why:** TUI was the largest scope item in the first-pass draft and the source of all read-frontier pressure (the ~600 file reads/sec analysis). Cutting it removes ~one child issue, a whole crate (`octl-tui`), two dependencies (`ratatui`, `crossterm`), and the entire polling/refresh design surface. Post-MVP a TUI can be added as a separate binary or subcommand reading the same canonical state — the schema is the contract, the TUI is the renderer.

**Trade-offs accepted:**
- Human navigation is uglier in MVP — `orchestratectl run list --json | jq` rather than j/k cursor movement.
- The "Miller-column / Finder mental model" that was a stated UX goal moves to a future phase.

## 4. Concurrency and safety

- Per-run advisory `flock` on `<run-dir>/.lock` for any writer.
- Reads do not lock; they tolerate a torn `manifest.json` (atomic rename means a reader either sees the old or new file, never a partial).
- `events.jsonl` is append-only; the `seq` counter is read from the last line under the lock, then incremented.
- Within a per-spawning-agent supervisor (see §7), the in-process write path holds the same `flock` for its mutations; short-lived CLI calls (skill-shim, manual `event create`) acquire and release the same `flock` per call. Both paths are correct because the lock is on disk, not in a process.

## 5. Crate layout

```
orchestratectl/
├── Cargo.toml
└── crates/
    ├── octl-core/        # schema types, file I/O, locking, event create, supervisor protocol
    └── octl-cli/         # clap-based CLI, --json plumbing, supervisor entry-point
```

Top-level `Cargo.toml` is a workspace; `cargo install --path crates/octl-cli` produces the `orchestratectl` binary. The `orchestratectl supervise <run-id>` subcommand re-enters the same binary as a supervisor process — no separate binary needed.

## 6. Dependency picks (MVP)

| Concern | Pick | Rationale |
|---|---|---|
| CLI parsing | `clap` v4 with derive | Standard, AI-first-friendly help output |
| Serialization | `serde` + `serde_json` | Schema is JSON |
| ID generation | `ulid` | Sortable, no coordination |
| Filesystem locking | `fs4` | Cross-platform advisory file locking (`flock` on Unix); maintained `fs2` successor — `fs2` is unmaintained with known soundness issues |
| Process supervision | `std::process::Command` + `nix` (`waitpid(WNOHANG)`, `kill(pid, 0)` for liveness checks) | Supervisor lifecycle. **Does not install a global SIGCHLD handler** — that would conflict with `std::process::Command`'s own internal reap path (causes `ECHILD`). Instead, liveness is polled (see §7.5). |
| Signal handling | `ctrlc` for SIGINT/SIGTERM trapping on the supervisor/event-tail processes | Lightweight, does not interfere with child reap. SIGINT → exit 130, SIGTERM → exit 143 per AGENTS-AI-FIRST-CLI §12. |
| Errors | `anyhow` (CLI), `thiserror` (core) | Library/binary split |
| Logging | `tracing` + `tracing-subscriber` JSON layer | JSONL log output requirement |
| Tests | `insta` for snapshot, `tempfile` for fixtures | Schema is easy to snapshot |

TUI dependencies (`ratatui`, `crossterm`) intentionally **dropped** with the TUI scope removal.

## 7. Process model (Fork 2)

**Decision:** Recursive per-run supervisor. Every run gets its own `orchestratectl supervise <run-id>` long-lived process — including leaf-style runs (a `spinoff` with no own children) because the supervisor is the home for that run's agent watchdog, lifecycle management, and `run cancel`/`run reattach` handling. Children's supervisors are spawned by the **parent** supervisor (not by the short-lived CLI), driven by `child.spawned` events that the CLI writes to the parent run's event log. Short-lived CLI calls (skill-shim, manual `discussion resolve`, etc.) write directly to the filesystem under `flock` — supervisors are not on the data path for these.

**Why:** A supervisor is the only entity that can:
1. Consume the child's terminal `node.report` event with **at-least-once-with-deterministic-dedup** semantics (see §7.8) — short-lived CLI cannot, because nothing is listening.
2. Act on the report (spawn approved spin-off runs, mark child done, surface to parent).
3. Run a watchdog for the agent process via PID + tmux-window polling (see §7.5) — `kill(agent_pid, 0)` + `tmux list-windows`. SIGCHLD is **not** usable because `create.sh` launches the agent inside a detached tmux window via workmux, so the agent reparents to PID 1 and the supervisor cannot `waitpid()` on it.

The "self-monitoring scripts" pattern that the existing `/orchestrate` skill writes today is essentially this supervisor — orchestratectl just standardizes it as a typed Rust process.

**Trade-offs accepted:**
- Lifecycle complexity per run (spawn/reap/orphan-detection). Bounded per-run, no global daemon to install.
- **Recursive supervisor count.** A fan-out with N direct children creates N+1 supervisors (one parent + N children); each child's own descendants similarly. At Jari's stated peak ~100 concurrent agents this is ~100 supervisor processes alongside ~100 tmux-hosted agents. Each supervisor is small (~5 MB resident, polling at 500 ms tick), so theoretical budget is well under 1 GB and CPU budget is well under 5% — but this is **empirically validated** in [`validation.md`](validation.md), not assumed. If empirical measurement shows pain, the optimization is supervisor consolidation (parent supervisor handles its direct children's report consumption without a separate process per child), but we do **not** pre-optimize.
- After machine reboot, stale `supervisor.pid` files exist for runs whose supervisors are gone. Reattach is explicit via `orchestratectl run reattach <run-id>` (§2.1, §7.6). The first read of such a run via `run show` reports the stale state honestly; it does not silently revive.
- Skill-shim path remains independent: bash code shelling `orchestratectl event create` does not require a live supervisor — the lock is filesystem-resident. Note: report **consumption** does require the parent supervisor; if the parent is dead, reports queue on disk until reattach.

### 7.1 Data flow: filesystem is the wire

Data between parent and child supervisors flows **through the shared `events.jsonl`**, not through inter-supervisor RPC. A child supervisor's appends to `events.jsonl` are observed by the parent supervisor's own tail-follow loop (`--from-seq <last_seen>`). No socket is needed for data; the only IPC is parent spawning child via `fork+exec`.

This means supervisors are **process-lifecycle managers + report consumers**, not data routers. They can die and be respawned (via `run reattach`) without losing data, because the canonical store is on disk.

### 7.2 Spawn protocol

The CLI invocation `orchestratectl run create --parent-run-id <P> --parent-node-id <PN> ...` (called by an agent from inside a worktree to spawn a child) is the trigger. The protocol is:

1. **CLI** validates inputs and the `--idempotency-key` if present. If the key was already used for this `(parent_run, parent_node)` combination, returns the prior `{run_id, dir}` and exits 0.
2. **CLI** generates the new child `run_id` (ULID) and a `c-<short-ulid>` short identifier for the parent's `children` registry.
3. **CLI** acquires the **parent run's** `flock`, appends `child.spawned {child_run_id, child_kind, child_title, idempotency_key, ...}` to parent's `events.jsonl`, updates the spawning node's `children` field via the reducer, fsyncs, releases parent lock.
4. **CLI** acquires the **child run's** `flock` (creates the directory + lock file first), writes `run.created` + `node.created` to child's `events.jsonl`, writes `manifest.json` and `nodes/n-0001.json`, fsyncs, releases lock.
5. **CLI** prints `{schema_version, run_id, dir, supervisor_pid: null}` and exits 0 **without spawning a supervisor**.
6. The **parent supervisor**, in its own tail-follow loop, sees the `child.spawned` event. It checks its in-memory "supervisors I've spawned" set for the `child_run_id`; if absent, it `fork+exec`s `orchestratectl supervise <child_run_id>`, records the resulting `supervisor_pid` in the child's `nodes/n-0001.json` via a small follow-up write, and adds to its tracking set.
7. The new **child supervisor** boots from disk: reads its manifest + node, registers `supervisor.pid`, runs `create.sh --type <kind>` to materialize the worktree + tmux window + agent, parses `create.sh`'s structured stdout (§8) to record `agent_pid` + `tmux_window` + `worktree_path` + `branch`, writes `supervisor.started` event, and enters its main loop (tail its own events + poll its agent's liveness).

**Top-level runs** (no `--parent-*` flags) skip step 3 and 6 — the CLI itself spawns the supervisor directly because there's no parent to delegate that to.

**Why the parent spawns the child supervisor rather than the CLI:** single source of authority. If the CLI spawned the supervisor, a duplicate `run create` (retry, race) could create two competing supervisors. By making spawn-of-supervisor the parent's responsibility, the parent's in-memory tracking set is the single arbiter — exact-once supervisor spawn is automatic.

### 7.3 Decision report (`node.report`) protocol

When the agent inside a worktree completes its task, it writes its terminal decision report:

```json
{"ts":"...","seq":N,"kind":"node.report","run_id":"<run-id>","node_id":"<node-id>","data":{
  "success": true,
  "summary": "Investigated the redirect; root cause is X. Implemented fix in commit abc123.",
  "discussion_items": [
    {"topic":"...", "severity":"discuss|critical", "options":["..."]}
  ],
  "spinoff_proposals": [
    {"proposed_title":"...", "proposed_kind":"spinoff", "rationale":"..."}
  ],
  "wrap_up_recommendations": [
    "Consider also touching X in a follow-up.",
    "..."
  ]
}}
```

The agent (typically Claude Code via its skill prompt) writes this via `orchestratectl node report --from-file <path> [--idempotency-key <opaque>]` which internally appends the event and updates `last_report` on the node JSON atomically under the run's `flock`. The `--idempotency-key` defends against the agent retrying the report submission. After fsync, the agent exits.

**Reliability protocol (at-least-once consumption with deterministic dedup):**

1. Agent writes `node.report` under `flock`, fsyncs, exits the tmux pane.
2. Parent supervisor learns the agent's run terminated via two signals it monitors:
   - (a) the new `node.report` event in its tail-follow loop on the child run's events;
   - (b) **the child agent's PID has exited** (detected by `kill(child.agent_pid, 0) == ESRCH`) **and** the child's tmux window is no longer in `tmux list-windows`.
   Processing happens only when **both (a) and (b)** are observed — guards against "agent crashed during fsync" (b without a) and "supervisor saw the report but child is still flushing" (a without b's PID exit). See §7.5 for the full liveness mechanic.
3. Parent supervisor processes the report **once per `(child_run_id, report_seq)` tuple**:
   - Mark child's root node `status: done` (or `failed` if `success:false`).
   - For each `spinoff_proposal[i]`, compute `proposal_id = "s-" + base32(sha256(child_run_id ":" child_node_id ":" report_seq ":" i)[:10])` and write `spinoff.proposed` to **parent run's** event log. If the `proposal_id` already exists in `spinoffs/<id>.json`, the write is skipped — this is how restart-recovery achieves dedup (§1.4).
   - For each `discussion_item[i]`, same pattern with `discussion_id`.
4. Parent supervisor updates its own `nodes/<spawning-node>.json` with `last_processed_report_seq_by_child[child_run_id] = report_seq`, fsyncs, releases lock.

If the supervisor crashes between writing some consumption events and recording `last_processed_report_seq_by_child`, the restart sees the same `report_seq`, computes the same deterministic IDs, and **idempotently skips** the writes for items that already exist. Replay is safe; no duplicate spinoff/discussion records appear.

### 7.4 Lifecycle: autonomous vs interactive

- **Autonomous lifecycle** (`spinoff`, `research`, `orchestrated`, `fan-out`, `bugfix`, `technical-decision`, `make-skill`): agent runs to completion, writes `node.report`, exits. Supervisor's watchdog (§7.5) fires `failed` if PID exit and tmux window disappear without a report event.
- **Interactive lifecycle** (`code`): agent waits for human input in tmux. May self-terminate (with a higher confirmation threshold inside its prompt) or be closed by the human via `orchestratectl run cancel` or by a `/worktree-merge`-style script that ends with calling `orchestratectl run cancel` or `orchestratectl node report` on the agent's behalf. Watchdog is lenient: long-lived agent process is normal; only an explicit terminal event closes the run.

From the caller (parent supervisor) perspective both lifecycles return the same way: a final `node.report` event arrives — either written by the agent itself or **synthesized by `run cancel`** (which emits a `node.report {success: false, reason: "cancelled by user", cancelled: true}` for any non-terminal node before exiting). This unification means parent supervisors never need lifecycle-aware logic; they wait for the report and act on it.

### 7.5 Agent liveness detection

Because `create.sh` (via workmux) launches the agent in a detached tmux window, the agent process reparents to PID 1 and the supervisor cannot `waitpid()` on it. SIGCHLD-based notification does not work for the agent. Instead the supervisor uses **dual polling**:

1. **PID liveness via `kill(agent_pid, 0)`**: returns `0` if the process exists, `ESRCH` if not, `EPERM` if it exists but is owned by a different uid. On macOS and Linux this works regardless of reparenting.
2. **PID identity defense**: if `kill(pid, 0)` returns `0`, the supervisor additionally checks the process start time against the stored `agent_pid_start_time`. If they differ, the PID has been recycled by an unrelated process and the agent is considered dead. macOS: `sysctl({CTL_KERN, KERN_PROC, KERN_PROC_PID, pid})` returns `kinfo_proc.kp_proc.p_starttime`. Linux: parse `/proc/<pid>/stat` field 22 (jiffies since boot, normalized to wall-clock).
3. **Tmux window presence via `tmux list-windows -F '#{window_name}'`**: confirms the agent's tmux window still exists. A live PID with the window torn down (e.g., user `tmux kill-window`) indicates a half-state; supervisor treats this as terminal and emits failed.

The agent is considered **alive** iff (PID alive AND start-time matches AND tmux window present). The agent is considered **dead** when both PID and tmux window are gone. Half-states (PID alive but tmux gone, or vice versa) resolve via short retry then commit to dead.

**Polling cadence**: 500 ms ticks during the agent's active phase, backoff to 2000 ms after 60s of no event activity to reduce idle CPU.

**`node.heartbeat` (optional)**: for very-long-running interactive agents, the agent skill may periodically emit `orchestratectl event create --kind node.heartbeat`. The supervisor records `last_heartbeat_at`; absence of both heartbeat and PID for > N minutes flags the node as `stalled` in projections (not failed; the human can decide). MVP does not mandate heartbeat for any kind; it's a future opt-in.

### 7.6 Supervisor restart / reattach

If the supervisor process dies (crash, kill, machine reboot), the run becomes "dormant on disk". The first observer (a `run show` CLI, a TUI, the human) sees `supervisor.pid` is stale — verified by the PID-identity check (§7.5) applied to the supervisor PID.

`orchestratectl run reattach <run-id>` restarts the supervisor:

1. Verifies the run exists and that `supervisor.pid` is stale (refuses to reattach a live run).
2. Acquires the run's `flock`, writes `supervisor.exited {reason: "stale on reattach"}` for the prior incarnation, releases lock.
3. Spawns a new `orchestratectl supervise <run-id>` process.
4. The new supervisor boots from disk:
   - Reads its `manifest.json` and all `nodes/*.json`.
   - For each `child` in the root node's `children` field, opens a tail-follow loop on that child's `events.jsonl` starting from `last_processed_report_seq_by_child[child_run_id]` (or `0` if absent).
   - For the local agent: probes liveness per §7.5. If alive, resumes watchdog. If dead, emits the failed status event.
   - Writes `supervisor.started {reattached: true}` event.

Replay during the reattach uses the deterministic-ID mechanism (§1.4, §7.3) so any prior consumption events are not re-emitted as duplicates.

### 7.7 Cancel synthesizes a terminal report

`orchestratectl run cancel <run-id>` is a domain verb (§2.0). Beyond marking `status: cancelled`, it **synthesizes a `node.report` event** for every non-terminal node in the run:

```json
{"kind":"node.report","node_id":"<node-id>","data":{
  "success": false,
  "reason": "cancelled by user",
  "cancelled": true,
  "summary": "Run cancelled before agent reported.",
  "discussion_items": [], "spinoff_proposals": [], "wrap_up_recommendations": []
}}
```

This guarantees parent supervisors waiting on a terminal report do not hang. Cancellation cascades to children via a `cancel` propagation step: the run-cancel CLI walks `children` recursively and emits `cancel` events into each subtree (or, post-MVP, the parent supervisor handles cascade as part of its own cancel handling).

### 7.8 Supervisor signal handling

The supervisor process is a long-running operation per AGENTS-AI-FIRST-CLI §12:
- Traps `SIGINT` (exit code 130) and `SIGTERM` (exit code 143) using the `ctrlc` crate (which does NOT install a SIGCHLD handler — that would conflict with `std::process::Command`'s internal reap).
- On signal: writes `supervisor.exited {reason: "signal", signal: SIGINT|SIGTERM}` event to `events.jsonl`, removes `supervisor.pid` if it still owns it, and exits.
- If `--format=jsonl` mode is used for supervisor invocations that stream live progress, emits a final `{"event":"cancelled","schema_version":1}` event on stdout before exit.
- A SIGTERM'd supervisor does **not** terminate its agent — the agent continues in tmux, and on `run reattach` the watchdog picks it up again.

## 8. Spawning — shell-out to create.sh (Fork 5)

**Decision:** In MVP, the supervisor's `spawn` step delegates to `~/.claude/skills/worktree/scripts/create.sh` with the appropriate `--type` argument. Post-MVP, the ~150 lines of `create.sh` logic can be ported into Rust as `octl-core/src/spawn.rs` without changing the on-disk contract.

**Why:** The skill family's tmux naming convention (emoji + branch name), workmux integration, agent command resolution, and `--layout` handling already live in `create.sh`. Shelling out keeps a single source of truth during the coexistence period — `orchestratectl run create --kind code` and `/worktree-code` produce identical windows and worktrees. No duplication, no drift.

**Trade-offs accepted:**
- Runtime dependency on `~/.claude/skills/worktree/scripts/create.sh` being present and readable by the user running `orchestratectl`. Documented in install instructions; supervisor errors with a clear diagnostic if the script is missing.
- Subprocess spawn cost (~50 ms per `create.sh` invocation). Negligible because spawn happens at most a few times per minute in normal operation.
- workmux remains an external CLI dependency; `create.sh` calls it. Pure-Rust workmux replacement is out of MVP scope (and workmux is third-party, not Jari's code, so library-mode embedding is not viable).
- **Cross-project dependency**: `create.sh` itself must be patched to emit structured stdout (§8.1). The patch is small (~10 lines of bash) and benefits both the skill family and orchestratectl; tracked as a `validation.md` dependency that must land before `breakdown.md` issue 10 (`all-kinds-spawn`) can be implemented.

### 8.1 create.sh contract (cross-project agreement)

For the supervisor to parse what `create.sh` did, the script must expose a structured stdout contract. The script's current human-readable echoes move to stderr; stdout becomes a single JSON object emitted on success:

```json
{
  "schema_version": 1,
  "type": "spinoff",
  "branch": "wt/01jx-n0001-login-redirect",
  "worktree_path": "/Users/jari/Sources/orchestratectl.worktrees/01jx-n0001",
  "tmux_window": "🚀 wt/01jx-n0001-login-redirect",
  "agent_pid_hint": 47821,
  "workmux_session": "orchestratectl",
  "tmux_socket": "/private/tmp/tmux-501/default",
  "tmux_session": "orchestratectl",
  "tmux_window_id": "@42"
}
```

- `agent_pid_hint` is what `workmux add` / `tmux send-keys` thinks the launched process PID is. It is a **hint**: tmux reparenting means the supervisor may need to do its own re-discovery (e.g., via `tmux list-panes -F '#{pane_pid}'` for the window) if `agent_pid_hint` is stale by the time the supervisor reads it. Stored in `nodes/<n>.json` as `agent_pid` after re-verification.
- **Qualified tmux identity** (`tmux_socket`, `tmux_session`, `tmux_window_id`) — added for the watchdog's liveness probe. The bare `tmux_window` *name* is not unique across sessions, and a `tmux list-windows -a` on the default socket is blind to windows on other sockets/servers — so a name-only match yields both false-positives ("agent alive" when it's a different session's same-named window) and false-negatives ("agent dead" on a non-default socket). The fix pins the window by its stable, per-server-unique `tmux_window_id` (`@NNNN`, captured after rename) on its own server `tmux_socket` (`#{socket_path}`, read from the created window). `tmux_session` is recorded for human display only — the watchdog does **not** scope by it, so a `rename-session` cannot break the match. The supervisor folds these into `nodes/<n>.json` as `tmux_identity` (`{socket, session, window_id}`), and `watchdog::probe_window_qualified` matches `tmux [-S <socket>] list-windows -a -F '#{window_id}'` (all windows on the server) for the recorded `window_id`. The probe is tri-state: a definitive absence (server answered, window gone) flips the node to `TmuxGone`; an inconclusive result (server unreachable / tmux missing / non-zero exit) is `Unknown` and leaves the verdict to the PID liveness check, so a wrong/dead socket cannot falsely reap a live agent. **Back-compat:** all three fields are optional. A create.sh that predates them — or that emits a partial/empty identity — gives `tmux_identity: null`, and the watchdog falls back to the legacy bare-name match (with a one-time warning). `tmux_window` is retained for human display.
- Exit codes follow AGENTS-AI-FIRST-CLI §2 conventions: `0` success, `1` user error (invalid `--type`, branch name conflict), `2` system error (workmux missing, tmux unavailable, partial side effect — see below).
- **Partial failure recovery**: if `create.sh` made any side effect (worktree exists, branch was created, tmux window was opened) before failing, it must clean up before exiting non-zero. If cleanup itself fails, exit `2` and write a structured error to stderr (`{schema_version, error: {code, message, partial_state: [...]}}`). The supervisor on `2` invokes its own cleanup pass and emits `node.status: failed reason: "create.sh exited non-zero, partial state: ..."`.
- Errors on stderr always follow the standard error envelope (`{schema_version, error: {code, message, invalid_value?, expected?}}`).

## 9. Open questions tracked in validation.md

All material design questions raised during the multi-LLM review pass have been resolved into this design with the following empirical or cross-project dependencies still pending validation. They are tracked in [`validation.md`](validation.md):

- **Supervisor process count at peak.** Theoretical estimate: ~100 supervisors at ~5 MB each = ~500 MB resident, polling at 500 ms = ~2 wakeups/sec/supervisor = ~200 wakeups/sec total. Measure on real Apple Silicon laptop; if numbers are 2× theoretical, document; if 10×, revisit consolidation.
- **`fs2` flock behavior on macOS APFS under contention.** Measure 10–50 concurrent writers, latency distribution.
- **`create.sh` structured-stdout patch.** Cross-project dependency. Patch must land in the skill family repo before `breakdown.md` issue 10 can ship. Coordinate via `validation.md`.
- **PID start-time identity check** on macOS and Linux: verify the `sysctl` / `/proc` paths return stable values across the hardware/OS combinations in use.
- **`tmux list-windows` polling cost** at peak (100 windows): measure tmux server CPU impact of polling every 500 ms.
- **Schema migration story (`schema_version: 1 → 2`).** Defer; design when v2 becomes concrete.
