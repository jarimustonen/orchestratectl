# taskfleet MVP — Breakdown

Child issues that deliver the epic. Each is sized to ~one session of focused work. Dependencies drawn from `design.md`.

## Critical path

```
cargo-scaffolding              create-sh-structured-stdout (cross-repo)
        │                                       │
        ▼                                       │
state-schema-crate ─────┬─────────────┬──────────────┐
        │               │             │              │
        ▼               ▼             ▼              ▼
run-cli-read     event-tail-cli   node-cli-read   event-create-cli
        │               │             │              │
        └───────┬───────┴─────────────┘              │
                ▼                                     │
        supervisor-process ◄──────────────────────────┘
                │
                ▼
        all-kinds-spawn ──► discussion-cli ──► spinoff-proposal-cli
            (also needs create-sh-structured-stdout)

(parallel utilities, not on critical path)
        version-subcommand   skill-subcommand
```

The critical path runs through `cargo-scaffolding` → `state-schema-crate` → read-CLIs (parallel) → `supervisor-process` → `all-kinds-spawn`. The `create-sh-structured-stdout` cross-repo patch is a hard prerequisite for `all-kinds-spawn` and runs in parallel with the Rust work. Discussions and spin-off CLIs are downstream of the state schema and can land in parallel with the spawn work. `version-subcommand` and `skill-subcommand` are MVP-essential AI-first plumbing but block nothing — they can land any time after scaffolding.

The first-pass draft's TUI issue is removed (Fork 4 — TUI deferred to post-MVP). The supervisor process is a new MVP issue that wasn't in the first pass; it's where the recursive supervisor model (Fork 2) materializes. The `version-subcommand` and `skill-subcommand` issues are required by the updated AGENTS-AI-FIRST-CLI.md (§10, §15).

## Child issues

| # | Slug | Type | Depends on | Summary |
|---|------|------|------------|---------|
| 1 | `cargo-scaffolding` | chore | — | Workspace `Cargo.toml`, two crates (`taskfleet-core`, `taskfleet-cli`), CI-friendly defaults, AI-first CLI plumbing: `--json` everywhere with `schema_version: 1`, JSONL log subscriber via `tracing`, stderr error envelope (`{schema_version, error: {code, message, invalid_value?, expected?}}`), warnings array on stdout JSON payloads, exit codes `0`/`1`/`2` per §2 of AGENTS-AI-FIRST-CLI. No `taskfleet-tui` crate. |
| 2 | `state-schema-crate` | feature | 1 | `taskfleet-core`: schema types (manifest, node, event, discussion, spinoff), atomic write helpers, per-run `flock`, event append primitive + `seq` counter. Includes `lifecycle: autonomous \| interactive`, `parent_run_id`/`parent_node_id`, and all 8 kinds in the enum. Snapshot tests against fixture runs. State files carry their own `schema_version` (starting at `1`) separate from CLI output `schema_version`. |
| 3 | `run-cli-read` | feature | 2 | `taskfleet run create\|list\|show\|cancel\|reattach`. `create` initializes a run dir; top-level vs child-spawn behavior per `design.md` §7.2 (top-level spawns supervisor; child-spawn writes `child.spawned` to parent and exits without spawning supervisor — parent supervisor does that from its tail-follow). `cancel` synthesizes terminal `node.report` events for non-terminal nodes (per §7.7). `reattach` restarts a stale supervisor and replays unprocessed reports (per §7.6). Implements `--dry-run` (with `dry_run_unsupported` on `create --parent-*` per AGENTS-AI-FIRST-CLI §11) and `--idempotency-key` (on `create`). (Note: verb is `create`, not `new`.) |
| 4 | `node-cli-read` | feature | 2 | `taskfleet node list\|show\|report`. `report` is a domain verb (§2.0 of `design.md`); ingests a JSON file matching the §7.3 payload spec and emits `node.report` event under `flock`. Implements `--dry-run` and `--idempotency-key` (defends against agent retry storms). |
| 5 | `event-tail-cli` | feature | 2 | `taskfleet event tail` with `--from-seq`, `--follow`, separate `--format=text\|json\|jsonl`, and `--output FILE` for batch capture (poll-based tail; no inotify in MVP). Without `--follow`: emits terminal `{"event":"result"}` on natural EOF. With `--follow`: traps SIGINT (exit 130) / SIGTERM (exit 143) per AGENTS-AI-FIRST-CLI §12; emits final `{"event":"cancelled"}` on signal. |
| 6 | `event-create-cli` | feature | 2 | `taskfleet event create --kind --node-id --from-file [--idempotency-key]` — sanctioned write path for skill-shim and external bash tools. Must run the reducer to update affected projection files (`manifest.json`, `nodes/*.json`, etc.) atomically within the same flock window (per `design.md` §2.3). Validates `kind` against the known event-kind set; unknown rejected. Implements `--dry-run`. |
| 7 | `version-subcommand` | feature | 1 | `taskfleet version [--json]` returning `{schema_version, version, commit, state_schema_version, supported_state_schemas}`. Per AGENTS-AI-FIRST-CLI §10 — agents need to detect drift between trained expectations and actual binary. Cheap; can land any time after scaffolding. |
| 8 | `skill-subcommand` | feature | 1 | `taskfleet skill list\|show\|install` — companion-skill installer per AGENTS-AI-FIRST-CLI §15. Skill files live under `crates/taskfleet-cli/skills/` and ship with the binary. MVP ships the subcommand + mechanics + 2 seed skills (`taskfleet-run-overview`, `taskfleet-spawn-spinoff`). Full skill library (replacing `/worktree-*`) is post-MVP. Cheap; can land any time after scaffolding. |
| 9 | `supervisor-process` | feature | 2 | `taskfleet supervise <run-id>` long-lived subcommand. Tail-follow loops on (a) own run events, (b) each child run's events (from `children` registry). On `child.spawned` event in own log: `fork+exec` a child supervisor for the new child run, record `supervisor_pid` in child node JSON, add to tracking set. On `node.report` in a child run's log: process per §7.3 with deterministic-ID dedup. Agent liveness via dual polling (`kill(agent_pid, 0)` + tmux window presence + start-time identity defense) per §7.5. **No global SIGCHLD handler** — would conflict with `std::process::Command`. SIGINT/SIGTERM trap (via `ctrlc` crate) with `supervisor.exited` event + clean PID file removal. Records `supervisor.pid`. |
| 10 | `all-kinds-spawn` | feature | 3, 9, 13 | `taskfleet run create --kind <X>` for all 8 kinds: validates `kind`, sets `lifecycle` from a kind→lifecycle table, calls `~/.claude/skills/worktree/scripts/create.sh --type <kind>` with right args, **parses structured JSON stdout** (per `design.md` §8.1) to extract `agent_pid_hint`, `tmux_window`, `worktree_path`, `branch`, re-verifies `agent_pid` via tmux pane PID lookup, registers the node, returns `{schema_version, run_id, dir, supervisor_pid}`. Replaces the first-pass `spinoff-spawn` issue and generalizes it. Depends on issue 13 (`create.sh` structured-stdout patch) landing first. |
| 11 | `discussion-cli` | feature | 2 | `taskfleet discussion list\|show\|resolve` (`resolve` is a domain verb, §2.0). Mutation writes `discussion.resolved` event, updates JSON under `flock`. Implements `--dry-run`. |
| 12 | `spinoff-proposal-cli` | feature | 2 | `taskfleet spinoff list\|approve\|reject` (domain verbs, §2.0). `approve` optionally calls `issuectl new` to materialize an issue. Implements `--dry-run`. |
| 13 | `create-sh-structured-stdout` | chore (cross-repo) | — | **Cross-project dependency.** Patch `~/.claude/skills/worktree/scripts/create.sh` to emit a single JSON object on stdout (per `design.md` §8.1): `{schema_version, type, branch, worktree_path, tmux_window, agent_pid_hint, workmux_session}`. Human-readable echoes move to stderr. Exit-code contract: 0 success, 1 user error, 2 system error with structured error envelope on stderr. Partial side-effect cleanup on failure. ~10 lines of bash; benefits the existing skill family too (callers get parseable output instead of regex-scraping). Lands in the homebase repo where `create.sh` lives. Must land before issue 10 can ship. |

## Out of scope for MVP (tracked for later)

- `tui-minimum-navigation` — three-pane Miller-column ratatui UI. Moved to post-MVP. Once added, it reads the same canonical schema; no protocol change required.
- `orchestrate-dag-runner` — native Rust DAG executor. MVP-time orchestration lives in the `/orchestrate` skill prompt; the agent recursively calls `taskfleet run create`.
- `fanout-runner` — native Rust parallel-units concurrency manager. MVP-time fan-out lives in the `/fan-out` skill prompt; the agent recursively calls `taskfleet run create --kind spinoff --worktree-type fan-out`.
- `skills-shim` — make `/worktree-*` skills write into `~/.taskfleet/runs/` so old and new coexist. Lands as soon as MVP is stable enough to be trusted alongside skills. **Note:** the `skill-subcommand` issue (#8) ships the *delivery mechanism* for skill files; the *full skill library replacement* of `/worktree-*` is this follow-up.
- `config-subcommand` — `taskfleet config path\|show` per AGENTS-AI-FIRST-CLI §8. Deferred until we have more than 1–2 configurable keys (currently only `$TASKFLEET_HOME`).
- `import-existing-tmux` — adopt pre-existing emoji-prefixed tmux windows into a synthetic run.
- `rebuild-projections-cli` — explicit replay-from-events tool. Schema is event-sourcing-clean by construction; tooling waits for actual need.
- `schema-migration-v2` — once `schema_version` changes.
- `macos-native-ui` — host UI consuming the same schema.
- `native-spawn-rust` — port `create.sh` shell-out into native Rust code in `taskfleet-core/src/spawn.rs`.
- `help-json-machine-readable` — `<subcommand> --help --json` structured help per AGENTS-AI-FIRST-CLI §14. clap supports the basics; full §14-compliance (examples array, env-var mappings) is a polish pass after MVP CLI stabilizes.

## Sequencing notes

- Issues 3–8 can land in parallel once 2 is in. `version-subcommand` (7) and `skill-subcommand` (8) depend only on scaffolding (1).
- Issue 9 (`supervisor-process`) and issue 10 (`all-kinds-spawn`) are the riskiest — they touch process lifecycle, signal handling, and the `create.sh` shell-out boundary. Worth their own `validation.md` checks before coding (see [`validation.md`](validation.md)).
- Issues 11 and 12 are mostly schema + CLI plumbing; cheap once 2 and the read CLIs are in.
- The lack of a TUI issue is a big simplification — the first-pass draft's "TUI parallelizable with spawn" comment is moot.

## Resolution of design open questions

The open questions in `design.md` §9 are tracked in `validation.md` and resolved during the supervisor-process and all-kinds-spawn implementation work. They do not block any MVP child issue from starting.
