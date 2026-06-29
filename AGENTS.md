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
