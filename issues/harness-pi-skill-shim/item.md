---
created: 2026-08-11
updated: 2026-08-12
type: feature
status: done
priority: normal
labels: [from-homebase-research]
closed: 2026-08-12
---

# pi worker bundled-SKILL prompt translation shim

## Description

Follow-up to run-create-harness-flag. run create --harness pi now launches a pi agent in the worker pane, but the bundled SKILL workflows the worker runs are Claude-Code-flavored (Skill/Agent tools, /worktree-merge slash commands, sub-agents/MCP — none of which pi has). pi is AGENTS.md-native with the Agent-Skills standard. Translate the worker-facing prompt/SKILL surface so a pi worker can actually complete the spinoff/research loop (work -> orchestratectl run merge -> report). Most orchestration is already external (run merge is a plain CLI call pi can run via bash), so the gap is narrow: map the Skill/Agent-tool references. Scope: make ONE autonomous kind (research or spinoff) complete end-to-end under pi.

## Resolution (2026-08-12)

Chose **research** as the single translated kind. Implemented `harness::prompt::worker_prompt_preamble(harness, kind, run_id)` — a new `crates/octl-cli/src/harness/prompt.rs`. For `(pi, research)` it returns an operating-note preamble that `run create` (`resolve_prompt_file` in `create.rs`) prepends before the `--task` brief when materializing `<run-dir>/prompt.md`. The preamble:

- Tells the pi worker it is AGENTS.md-native and has none of Claude's Skill/Agent tools, sub-agents, MCP, or `/worktree-*` / `/llm-*` slash commands.
- Maps `/worktree-merge`, `/complex-rebase`, "self-merge" → the plain `orchestratectl run merge` bash; `/llm-review` / sub-agents → skip; any other `/name` → Claude-only, ignore.
- Carries the **self-contained closing bash** with the **exact run id templated in** (no `ls | grep` discovery — the in-code advantage over the static SKILL) and a **quoted heredoc** so a model-authored summary can't be shell-expanded.

Every other `(harness, kind)` returns `None`, so the claude path is byte-identical and un-shimmed pi kinds are left untranslated (explicit follow-up). `--prompt-file` stays caller-owned when there is no preamble; with a preamble the derived prompt is written into the run dir so the caller's file is never mutated.

Reviewed via `/llm-review` (4 models) + `/assess-findings` (`history/assessment-harness-pi-skill-shim.md`): applied the 4 unanimous/strong findings (run-id templating, quoted heredoc, placeholder-vs-exact wording, conflict re-run split). Green gate: fmt/clippy clean, workspace tests green (3 pre-existing tmux-under-load `supervise::capture` flakes pass in isolation).

**Done bar met:** research completes end-to-end under `--harness pi`. Out-of-scope follow-ups (other kinds' translation, effective-prompt provenance, pi e2e integration test) → report spinoff_proposals.
