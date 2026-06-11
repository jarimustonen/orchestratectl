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
