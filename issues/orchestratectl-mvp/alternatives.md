# orchestratectl MVP — Alternatives considered and rejected

For each of the five forks decided in `design.md`, this document records the alternatives that were seriously considered during the multi-LLM workshop (Phase 2 proposals from 4 frontier models) and the dialogue with the user (Phase 3). Each entry records the alternative, why it was rejected, and the conditions under which a future revisit would be warranted.

This document is descriptive, not prescriptive — its purpose is to make future "why didn't you just …?" questions answerable in seconds.

## Fork 1 — State persistence

**Chosen:** Per-run JSON projections + append-only `events.jsonl` source-of-truth + per-run `flock`. See `design.md` §1.

### A1.1 — Per-run SQLite (events table + projections, single transaction)

**Proposers:** gemini-3.1-pro-preview, deepseek-v4-pro, gpt-5.5, claude-opus-4-7 (all four).

**Strategy:** Each run directory contains one `state.db` SQLite file. The `events` table is the append-only log; `nodes`, `discussions`, `spinoffs` are normalized projection tables maintained via INSERT/UPDATE in the same SQL transaction. SQLite's own multi-process locking (WAL mode) replaces `fs2` flock.

**Why rejected:**
- Skill-shim shell scripts cannot append events with `echo ... >> events.jsonl`-style simplicity. They must shell to `orchestratectl` for every event. With the JSONL design this is also true (the design bans direct shell flock + echo per §1), so this difference is small, but SQLite makes "inspect events with `cat`/`jq`" impossible — only `sqlite3 state.db '.dump'` works.
- The user explicitly cited debuggability with `cat events.jsonl | jq` as a valuable property.
- The TUI was removed from MVP, eliminating the strongest argument for SQLite (efficient indexed reads over many rows).
- Cross-run queries (e.g., "every open discussion across runs") are awkward in SQLite (ATTACH dance) — same problem as JSONL, no upside.

**Revisit if:**
- Event volume grows past tens of thousands per run and `seq`-counter recovery (scanning to EOF on each append) becomes a measurable bottleneck.
- A future post-MVP fan-out runner needs typed queries like "which spinoff proposals have no resolution yet" across many runs.
- The validation V5/V6 results show that polling many JSONL files is too expensive at peak (>10× theoretical) — SQLite's indexed reads would close that gap.

### A1.2 — Hybrid: JSONL canonical + SQLite cache/index

**Proposers:** deepseek-v4-pro, gpt-5.5, claude-opus-4-7.

**Strategy:** `events.jsonl` remains canonical source-of-truth. A separate SQLite database (per-run or global) is maintained as a disposable, rebuildable cache for fast CLI/TUI reads. Stale → replay from JSONL tail.

**Why rejected:**
- Two storage layers means two failure modes (stale cache, drift bugs). With the TUI removed and only CLI reads in MVP, the cache's value proposition shrinks substantially.
- Adds significant complexity to every writer: "append event, update projection JSON, upsert cache row." Reasoning about which read sources trust which layer becomes a fertile bug area.
- The user explicitly favored a single source of truth model.

**Revisit if:**
- A future TUI is added and the polling cost is measurable.
- Cross-run query patterns become common enough to justify maintaining a derived index.

### A1.3 — Pure JSONL only (no projection JSON files)

**Proposer:** gemini-3.1-pro-preview (only).

**Strategy:** No `manifest.json`, no `nodes/*.json`. Every CLI read replays `events.jsonl` from `seq=0` and folds the state. TUI keeps in-memory state across ticks.

**Why rejected:**
- Every short-lived `run show` / `node show` would scan the entire event log. At any non-trivial run history this becomes painful — and `node show` is exactly the kind of frequent CLI call agents make.
- The user wants debuggable on-disk state for *both* events and projections; "current state of the node" should be inspectable with `cat nodes/n-0001.json | jq`, not derived in-process every time.
- Trade-off shape is not favorable: gains "zero divergence" but loses fast reads, with no compensating benefit at MVP scale.

**Revisit if:** Never likely. Projection drift turns out to be a real correctness problem that justifies the maximalist replay-everywhere model. Not foreseen.

## Fork 2 — Process model

**Chosen:** Recursive per-run supervisor process. Each run has its own supervisor; parents spawn children's supervisors from their tail-follow loop. See `design.md` §7.

### A2.1 — Short-lived CLI everywhere, no supervisor process

**Proposers:** All four models proposed this as a baseline; user initially favored it but converged away during dialogue.

**Strategy:** No long-lived process at all. TUI (if any) is the only long-lived reader. Agents are detached in tmux and write to disk; their reports sit in `events.jsonl` until a human runs `orchestratectl spinoff list` or similar.

**Why rejected:**
- The user's "päätösviesti" (decision report) requirement explicitly demands that some entity reliably acts on the child's terminal `node.report` event — spawning approved spinoffs, surfacing discussions, marking the parent's view. A short-lived CLI is not listening; the agent's report would queue indefinitely.
- Watchdog for stuck agents has no natural home in a stateless-CLI model.
- The user's mental model of "every spawning agent has its own supervisor" aligns naturally with the recursive supervisor design.

**Revisit if:**
- The decision-report acting becomes "user runs `orchestratectl process-reports` explicitly when they feel like it" — then the supervisor's role evaporates. Considered and rejected because that's worse UX.

### A2.2 — Single global daemon (`orchestratectld`)

**Proposers:** gemini-3.1-pro-preview, gpt-5.5, claude-opus-4-7.

**Strategy:** One long-lived `orchestratectld` process owns all runs, all locks, all child processes. CLI is a thin client over a Unix socket. Auto-started on first CLI invocation, supervised by launchd/systemd.

**Why rejected:**
- The user's hard constraint that "skill-shim must work without a daemon being up" required a fallback file-write path anyway. With both paths existing, the daemon becomes optional-but-also-the-source-of-truth, which is the worst of both worlds (two write paths to keep consistent).
- Single point of failure: daemon crash takes down all runs.
- The recursive per-run model maps directly to the user's mental model of "an agent that spawns its children supervises them" — a global daemon is the wrong shape.

**Revisit if:** Never likely for orchestratectl. The recursive supervisor design IS the per-run daemon, just bounded in scope.

### A2.3 — File-watcher daemon (optional, push-only) + stateless writes

**Proposer:** deepseek-v4-pro.

**Strategy:** Writes remain short-lived CLI shell-outs. A separate optional `orchestratectl watch` daemon subscribes to filesystem events via `kqueue`/`inotify` and pushes diffs to the TUI over a socket. Pure UX optimization.

**Why rejected:**
- TUI was removed from MVP. The watcher's entire purpose was TUI responsiveness.

**Revisit if:** Post-MVP TUI is added, and 500 ms polling proves visibly slow.

## Fork 3 — CLI surface

**Chosen:** Linear/git-style `<noun> <verb>` subcommand tree, strict AGENTS-AI-FIRST-CLI compliance (verb vocabulary `list|show|create|update|delete`, domain-verb exceptions documented). See `design.md` §2.

### A3.1 — Declarative manifest (`orchestratectl apply -f run.yaml`)

**Strategy:** Like Kubernetes / Terraform. Runs and their state are described in YAML/TOML manifests; `apply` converges to the described state.

**Why rejected:**
- AGENTS-AI-FIRST-CLI §6 explicitly restricts `apply` to cases where convergent reconciliation is real semantics, not aesthetic. orchestratectl creates one-shot runs with no convergence semantics — re-`apply`-ing a "create this run" manifest is just a duplicate.
- AI callers compose CLI calls from a planning step; one argv per call is the right granularity. Manifests split intent across argv + file content and add a stat/parse step for the agent.

**Revisit if:** Some future feature genuinely has reconciliation semantics (e.g., a long-term schedule of runs). Add `apply` then; do not retro-fit.

### A3.2 — Verb-first flat surface (`orchestratectl list-runs`, `orchestratectl spawn-spinoff`)

**Strategy:** Like older Unix tools; no noun layer, just verbs.

**Why rejected:**
- orchestratectl has many distinct resources (runs, nodes, events, discussions, spinoffs). A flat verb surface multiplies verbs (`list-runs`, `list-nodes`, `list-events`, `list-discussions`, `list-spinoffs`) and breaks predictability.
- AGENTS-AI-FIRST-CLI §6 explicitly says noun-verb is the AI-first default; flat verb-first is for single-resource tools (`cargo`, `npm`).

**Revisit if:** Never.

## Fork 4 — TUI

**Chosen:** No TUI in MVP. CLI + `event tail --follow` is the human view. See `design.md` §3.

### A4.1 — `ratatui + crossterm` three-pane Miller-column TUI

**Strategy:** The first-pass design's plan. Read-only TUI polling files at 500 ms.

**Why rejected:**
- TUI was the largest single scope item and the source of all polling-cost analysis (~600 reads/sec).
- Cutting it removes a crate (`octl-tui`), two dependencies, and an entire interaction model from MVP.
- Post-MVP a TUI can be added as a separate binary or subcommand reading the same canonical state — the schema is the contract.

**Revisit if:** After MVP ships and the user finds CLI navigation painful enough to justify the work. Likely; planned but not now.

### A4.2 — Alternative TUI stacks (`cursive`, raw `crossterm`, `egui` headless)

**Strategy:** Variations on the rendering library if a TUI were in scope.

**Why rejected:** TUI itself rejected at the layer above; choice of library is moot.

**Revisit if:** TUI returns to scope, evaluate then.

## Fork 5 — Worktree+tmux spawning

**Chosen:** Shell-out to `~/.claude/skills/worktree/scripts/create.sh` in MVP; native Rust port deferred post-MVP. See `design.md` §8.

### A5.1 — Native Rust via `git2` + `tmux` shell-out

**Strategy:** Use the `git2` crate for `git worktree add` (no shell-out for git), still shell out to tmux because no good Rust binding.

**Why rejected for MVP:**
- Duplicates emoji-prefix naming logic, workmux integration, layout selection, hooks handling — all of which already live in `create.sh`.
- Risk of drift between orchestratectl's Rust implementation and the existing skill family during the coexistence period.
- `create.sh` is ~150 lines of bash; native Rust port is straightforward post-MVP when MVP is proven.

**Revisit if:** Post-MVP — issue `native-spawn-rust` is tracked for this.

### A5.2 — `tmux control-mode` (`tmux -C`)

**Strategy:** Supervisor opens a persistent control-mode connection to tmux; receives push notifications when windows die.

**Why rejected:**
- The watchdog (`design.md` §7.5) already uses `tmux list-windows` polling, which is sufficient at MVP cadence.
- Control-mode would add real complexity (long-lived tmux client connection per supervisor) that doesn't pay off without a TUI or higher-frequency liveness needs.

**Revisit if:** Validation V6 shows `tmux list-windows` polling overloads the tmux server. Control-mode would replace polling with push.

### A5.3 — No tmux at all (supervisor's direct child process)

**Strategy:** Supervisor `fork+exec`s the agent as its own child. No tmux, no detach, no workmux. The agent is a normal child process; SIGCHLD works; `waitpid` works.

**Why rejected:**
- Breaks the user's existing workflow of "open the tmux window to watch / interact with the agent". This is especially important for the `code` lifecycle, where the human drives the agent interactively.
- Loses workmux's session/layout features.
- Was tempting post-TUI-removal but the human-driven interactive kinds need tmux.

**Revisit if:** A future MVP variant adds a `--detached` or `--headless` kind that runs without tmux; supervisor could spawn those as direct children. Out of scope now.

### A5.4 — Embed `workmux` as a Rust library crate

**Strategy:** Depend on `workmux` as `Cargo.toml` library; no shell-out at all.

**Why rejected:**
- `workmux` is third-party (by `raine`, GitHub: raine/workmux), not the user's code. Its API is not part of a public stability promise.
- Library-mode embedding couples orchestratectl to upstream workmux releases; every upstream bump risks breakage.
- Realistic check shows the brew-installed workmux binary is Mach-O; even if Rust, the lib.rs surface (if any) is not designed for embedding.

**Revisit if:** Never likely. If workmux's bash-callable contract changes, the right response is to patch `create.sh` or write a thin Rust replacement — not embed workmux.

## Non-fork decisions made during workshop

These were decided in dialogue without being explicit "forks" but are worth recording for archeology.

### N1 — Lifecycle field on manifest (`autonomous | interactive`)

**Why introduced:** Distinguishes self-terminating kinds (spinoff, research, etc.) from human-driven kinds (code). Supervisor's watchdog and `run cancel` semantics differ.

**Alternative considered:** Hard-code lifecycle per kind in the supervisor's logic. Rejected because making it a manifest field makes future kinds easy to add without re-deploying the binary.

### N2 — Parent CLI writes `child.spawned` to parent's log; parent supervisor spawns child supervisor (not the CLI)

**Why this choice:** Single source of authority for "who owns this child". CLI cannot race with a duplicate `run create` because the parent supervisor's in-memory "supervisors I've spawned" set is the arbiter.

**Alternative:** CLI spawns the supervisor directly. Rejected because of the race: two concurrent `run create` calls with the same idempotency-key could both spawn a supervisor.

### N3 — Deterministic IDs from sha256(key tuple) for derived events

**Why this choice:** Idempotent restart-recovery comes for free; no scan-before-write needed.

**Alternative considered:** Scan parent's recent event history before each consumption-event write. Rejected because deterministic IDs are simpler and faster — duplicates are detected at the projection-file level (file already exists with matching content).

### N4 — `skill` subcommand in MVP (mechanics only, full skill library post-MVP)

**Why this choice:** AGENTS-AI-FIRST-CLI §15 requires the skill subcommand. Having the mechanics in MVP means the path is open for the eventual replacement of the `/worktree-*` skill family with orchestratectl-shipped skills.

**Alternative:** Defer entirely until ready to write the full skill library. Rejected because adding the subcommand later means orchestratectl's binary has a structural gap (no skill installer); building the mechanics now is small and forward-compatible.
