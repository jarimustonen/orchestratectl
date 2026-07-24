# orchestratectl

Rust CLI + TUI for orchestrating AI-agent workflows — worktrees, fan-out,
orchestrate, LLM-skills — with status navigation, discussion resolution,
and spin-off management.

Replaces (and runs alongside, until MVP is stable) the existing skill
family: `/worktree-*`, `/orchestrate`, `/fan-out`, and the orchestration
side of `/llm-*` skills. State is file-based under
`~/.orchestratectl/runs/<run-id>/` so any UI (TUI now, macOS-native later)
can present the same canonical source of truth.

## CLI Design Principles

This project follows the AI-first CLI conventions in [`AGENTS-AI-FIRST-CLI.md`](AGENTS-AI-FIRST-CLI.md) — strict input validation, `--json` output, JSONL logs, no interactive prompts, informative errors, composable commands. Read that file before designing or changing CLI surface. The file is a verbatim copy from `homebase`; treat it as shared canon, not a project-local doc to edit.

## Gitignored directories

- `history/` — agent scratchpad and ephemeral planning docs (not tracked)
- `target/` — Rust build artifacts

## Documentation Pattern

Every directory follows this structure:

- `CLAUDE.md` — symlink to `AGENTS.md`
- `AGENTS.md` — all AI-relevant info (consolidated)
- `AGENTS-<TOPIC>.md` — complex topics split out (optional)

## Issues & Planning

Issue tracking is managed by [`issuectl`](https://github.com/jarimustonen/issuectl). Use the `/issue` skill (installed by `issuectl init`) to create, search, update, and close issues.

- `issues/<slug>/item.md` — every issue and epic (flat layout — no numeric prefix, no `open/closed/` split)
- Status lives in the `status:` frontmatter field, not in the path
- `issues/AGENTS.md` — issue schema, types, workflow (owned by issuectl)
- `.issuectl/AGENTS.md` — repo-local policy for AI agents (owned by issuectl)

All planning documents (plans, analyses, validations, designs, breakdowns, todos) belong under their parent issue directory — not as standalone files. If work needs a planning document, it also needs an issue.

- `issues/<slug>/plan.md` — architecture, implementation plans
- `issues/<slug>/analysis.md` — research and analysis
- `issues/<slug>/validation.md` — design assumptions checked against current reality, noting what differs from first-pass analysis
- `issues/<slug>/design.md` — design documents
- `issues/<slug>/breakdown.md` — epic → child-issue breakdown with dependencies and critical path
- `issues/<slug>/todo.md` — task checklists

## Spinoff workflow + lifecycle

Use `/worktree-spinoff <issue-slug>` for bug fixes / improvements; the bundled SKILL handles the whole loop end-to-end: spawn → work → merge (`orchestratectl run merge`) → self-cleanup (tmux window + worktree + branch all gone). Same for `/worktree-research`, `/worktree-bugfix`, `/worktree-technical-decision`, `/worktree-make-skill`. Interactive `/worktree-code` works the same way but waits for the user's explicit `/worktree-merge`.

After any CLI surface or SKILL.template.md change, **re-deploy** so the running binary + on-disk skills reflect the edit:

```bash
cargo install --path crates/octl-cli --force
orchestratectl skill install --force
orchestratectl doctor   # confirms skill.sync.* ok for every entry
```

For parallel spawn batches, set up a `Monitor` watching `orchestratectl event tail <run-id> --follow` filtering `node\.report|run\.status|supervisor\.exited` so completions arrive as notifications instead of requiring polling.

### Never `pkill` a supervisor without verification

Twice in one session this rule was learned the hard way: `pgrep -lf "orchestratectl supervise"` finds processes from EVERY repo and every user-owned project, not just yours. Before killing anything:

1. Run `tmux list-windows -a` and look at the emoji prefix on each `wt-*` window — it identifies the source project (🏠 home, 🥨 dpad, 🎬 orchestratectl, etc.).
2. Run `git worktree list` in the **right repo** to see if the run's worktree is one yours.
3. Prefer `orchestratectl run cancel <run-id>` over `pkill` — graceful, triggers the supervisor's cleanup path, leaves no orphans.
4. If you must `pkill`, scope it: `pkill -f "orchestratectl__worktrees/.*supervise"` only kills supervisors built from inside a deleted worktree's debug target — never touches `~/.cargo/bin` production supervisors.

## macOS PTY constraint

macOS limits concurrent pseudo-terminals; ~5–6 simultaneous worktree spawns can hit `fork failed: Device not configured` from tmux. Symptom: `create.sh` fails with `workmux-add-failed` mid-batch.

Use `--headless` (or `--tmux-session <name>`) on `orchestratectl run create` to spawn into a detached tmux session that doesn't consume a foreground PTY. Mandatory for `/fan-out` of N≥5; recommended for any parallel `/worktree-spinoff` batch larger than 3. Attach later with `tmux attach -t headless` to inspect.

## State integrity invariants

These five invariants govern correctness of the on-disk run state and the autonomous-spinoff loop. They were established by the 2026-06-29 pre-publication campaign (`B1.1–B1.4`, `C1`, the in-session safety bugs) and are easy to violate from inside a hot code path without realising it. Read them before touching the reducer, the lock layer, or the `run merge` / supervisor cleanup paths.

1. **`applied_seq` watermark**
   (`crates/octl-core/src/events.rs`)
   The reducer advances `manifest.applied_seq` only after every projection an event touches has been fsynced. On the next lock acquisition, events with `seq > applied_seq` are replayed before any new append. Any new event-appending path MUST go through the `LockedRun` witness and the `append_and_apply_*` API — never call `write_*` projection helpers directly.

2. **`LockedRun` witness**
   (`crates/octl-core/src/lock.rs`)
   Compile-time proof that the caller holds the run flock before calling `append_event_with_seq` / `append_and_apply_unlocked`. Don't add `#[allow(...)]` to bypass; thread the witness through.

3. **`LOCK_SH` on every multi-file read path**
   (`crates/octl-core/src/lock.rs::with_shared_lock`)
   Every reader that touches more than one of `manifest.json` / `nodes/*` / `discussions/*` / `spinoffs/*` in one decision wraps the scan in `RunLock::with_shared_lock`. The reducer holds the exclusive lock while it writes; without the shared lock a reader can observe a half-applied projection set. Don't add new readers that skip it.

4. **Progress polling branches on `manifest.status`, NOT `lifecycle`**
   (every `crates/octl-cli/skills/*/SKILL.template.md`, and any agent prose elsewhere)
   `Lifecycle` is `Autonomous | Interactive` — a *category* derived from `kind`, never transitions. `Status` is `Pending | Running | Done | Failed | Cancelled` — terminal states are `Done | Failed | Cancelled`. An agent that polls `lifecycle` for `completed | failed | cancelled` hangs forever; the field never matches. This was a real bug (`skill-progress-polling-wrong-field`); never re-introduce it.

5. **Supervisor is the canonical worktree + tmux teardown actor**
   (`crates/octl-cli/src/supervise/cleanup.rs`)
   `merge.sh` no longer touches tmux or `git worktree remove` — the supervisor sees the terminal `node.report`, rolls the run up via `rollup_status`, and tears down. `find_window_by_path` is **session-scoped + exact-cwd-match**: it queries only the spawn-session via `tmux list-windows -t <session>` and requires `pane_current_path == worktree_path` (no sub-path prefix). Without these constraints the recovery would kill an unrelated pane that happened to `cd` into the worktree, including the user's master session.

   **Teardown is gated on the terminal outcome — unmerged work preserves the branch + worktree** (`node_report_is_blocked` + the source-relative check, issue `blocked-report-deletes-branch`). Two layers:
   - **Primary gate:** a node whose terminal `node.report` is a blocked handoff (`success: false`, no `via: "explicit-merge"`, not a `cancelled` run-cancel) committed work that was never merged. `cleanup_node` closes its tmux window (winding the run down is fine) but must NOT `git worktree remove` or delete its branch — it records a `cleanup.branch_preserved` audit event instead. Deleting them is silent data loss.
   - **Defense-in-depth (source-relative):** on ANY non-explicit-merge path (a plain success that skipped `run merge`, a `run cancel`, a genuine failure, a future ungated outcome), `cleanup_node` checks `git rev-list --count <manifest.source_branch>..<branch>` **before** touching anything. If the branch has commits not reachable from the run's OWN source branch, it preserves BOTH worktree and branch (`cleanup.branch_preserved`, reason `unmerged commits vs source`). The ancestry check is against the run's recorded source branch, NOT the main worktree's ambient `HEAD` (which may be on any branch when the supervisor ticks). This means a `run cancel` whose agent committed real work now preserves it too.
   - **Last-resort backstop:** only a confirmed `run merge` force-deletes (`git branch -D`); every other delete uses `git branch -d` (refuses an unmerged branch, ambient-HEAD-relative) for the residual case where `source_branch` was unrecorded and the source check could not run. Branch names are passed after `--`.

   **The `run create --notify` completion hook fires on the terminal transition, BEFORE teardown** (`crates/octl-cli/src/supervise/notify.rs`, issue `no-completion-notification-to-parent`). The order in the terminal tick is fixed: fire notify → cleanup → loop-exit, so a hook can observe the run before the worktree/window are gone. Delivery is **at-least-once** (owner's call: a missed completion signal is worse than a duplicate): under one exclusive lock the supervisor scans for a durable `run.notified` marker (idempotency key `supervisor-notify:<run-id>`, scoped by `(kind, key)`) and, if absent, spawns the hook FIRST and records the marker AFTER — so a crash between the two re-fires on restart. Do NOT reorder to record-before-spawn (that is at-most-once and silently drops the notification on a crash). `notify` state is tracked SEPARATELY from `cleaned` (a shared flag silently drops the notification on a transient append failure — a bug caught in review); don't re-merge them. The hook is spawned detached and reaped on a thread so a hung command can't wedge the single-threaded tick.

### Related conventions

- **Concurrent spinoff reports** — bundled SKILLs use `/tmp/node-report-${run_id}.json`, never the shared `/tmp/node-report.json`. Drift re-introduces the clobber race.
