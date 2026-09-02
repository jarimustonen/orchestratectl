---
name: worktree
description: Router for the `/worktree-*` family. Classifies a free-form request into the right variant (`/worktree-spinoff`, `/worktree-research`, `/worktree-technical-decision`, `/fan-out`) and delegates. Use when the user invokes `/worktree <free-form task>`, `/worktree <issue-slug>`, or `/worktree --flag ... <task>`, or says "spawn a worktree for X", "start a worktree to do Y", "do this in a worktree" without naming a specific variant. Does NOT create worktrees itself; does NOT route to `/worktree-merge` or `/complex-rebase` (those operate on existing worktrees — the user invokes them directly).
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Worktree Router

Arguments: `$ARGUMENTS`

You classify a free-form worktree request, announce the choice in one line, then delegate via the Skill tool. You never create a worktree, never read repo files, never rewrite the user's task.

## Hard constraints

1. **You do not create worktrees.** No `orchestratectl run create`, no `create.sh`, no `git worktree`, no prompt files. Every code path ends in delegation or refusal.
2. **You never route to `/worktree-merge` or `/complex-rebase`.** Both operate on existing branches/worktrees and have preconditions the router can't validate. If the user wants to merge or to do a non-mechanical rebase, tell them to invoke the skill directly.
3. **You never route to `/worktree` (yourself).** No recursion.
4. **You forward the user's arguments verbatim** to the chosen sibling. Do not strip flags, rephrase the task, summarize, expand, or infer additional context. The user's text + flags become `$ARGUMENTS` for the sibling; flag validation is the sibling's job.
5. **You delegate exactly once per call.** Pick one sibling, invoke it via the Skill tool, stop. No retry routing, no fallback chain on sibling failure.
6. **You do not silently fall back to the default `/worktree-spinoff` on ambiguity.** Clarify in one sentence first — see *Ambiguous input* below.
7. **You read only `$ARGUMENTS`.** No `Read`, `Glob`, `Grep`, no issue files, no git state inspection. If routing requires knowing something only the repo can tell you, ask the user.

## Routing table

Match `$ARGUMENTS` against the rows top-down; the first row whose signal fires wins. "Signal fires" means the user's prompt clearly expresses the intent in the left column — not mere substring match.

| Signal in the user's request | Delegate to |
|---|---|
| "decide whether X or Y", "make the architectural call", "settle the trade-off", ADR | `/worktree-technical-decision` |
| "research", "investigate", "survey" a topic of inquiry (multi-source synthesis; **not** a factual lookup, **not** a single-doc summary, **not** a bug to fix, **not** a decision request) | `/worktree-research` |
| 5+ identical independent units, "fan out", "for every X in <enumerated set>" | `/fan-out` |
| Default: a single focused task (coding, bugfix, or any autonomous work); no other signal fires | `/worktree-spinoff` |

An issue slug (`intensifier-adjective-noun`, e.g. `extremely-quiet-otter`) with no other directional signal is a single autonomous task → `/worktree-spinoff`.

## Out-of-family requests — refuse, do not route

If the request is about managing an existing worktree, looking something up, or asking a question, refuse with a one-line redirect. Do not invoke any skill.

| User wants | Tell them to invoke |
|---|---|
| Merge a worktree back | `/worktree-merge` |
| Rebase requiring re-implementation across diverged branches | `/complex-rebase` |
| List worktrees | `orchestratectl run list` (or `git worktree list`) |
| Prune dead worktrees | `git worktree prune` |
| Remove a specific worktree | `git worktree remove <path>` |
| Add a worktree by hand (no agent) | `git worktree add` |
| Ask a question about git worktrees / the family / a sibling | answer directly, do not create a worktree |
| Empty `$ARGUMENTS` or `--help` / `-h` | print a one-line family overview and ask for a task |

## Ambiguous input

Two siblings tied on real signals, or no signal fires beyond the default with the task framing genuinely unclear, or the prompt names two routes ("compare options and decide"): ask one conversational sentence and stop. Examples:

- "Is this one task or 5+ similar ones?" (→ `/worktree-spinoff` vs `/fan-out`)
- "Do you want a survey of options (`/worktree-research`) or a recorded decision (`/worktree-technical-decision`)?"

No `AskUserQuestion` — conversational text only (global CLAUDE.md). After the user answers, re-route per the clarified intent and **pass the full original task** to the sibling (the user's clarifying reply is routing context, not the new task body).

## Novel workflow — recommend a new sibling

If `$ARGUMENTS` describes something that genuinely doesn't fit any sibling, first ask one sentence: "Is this a one-off, or a workflow you'd want to invoke repeatedly?"

- **One-off** → after the clarification, route to `/worktree-spinoff` (the default autonomous worker). Do not propose authoring a skill for a one-off.
- **Repeatable** → name the two closest siblings, explain in one sentence why each doesn't fit, then suggest authoring a new `/worktree-<x>` variant with `/skill-creator`. Skill authorship is a separate decision — do not auto-invoke it.

## Announce and delegate

Before invoking the chosen sibling, emit exactly one line of routing output:

> Routing to `/worktree-<variant>` — <one-line reason>.

Then invoke that skill via the Skill tool, passing `$ARGUMENTS` verbatim. Examples:

- `Routing to /worktree-research — survey-of-options framing on an inquiry topic.`
- `Routing to /fan-out — "for every receipt in batch 2026-05" is 8 identical units.`
- `Routing to /worktree-spinoff — single focused task, default autonomous flow.`

Keep the line short; the user can interrupt if you misclassified.

## Anti-patterns

- Don't open files, run greps, or load issue text to plan an approach. Reading `$ARGUMENTS` itself is fine; reading anything else is not.
- Don't write prompt files, task descriptions, or any worktree scaffolding — that's the sibling's job.
- Don't ideate, expand, or "helpfully" infer additional context about the user's task before forwarding.
- Don't strip or "normalize" flags. `--with-test-server`, `--review`, `--headless`, `--deepseek-flash`, `--sparse foo,bar`, etc. are sibling-owned; forward as-is and let the sibling reject unknowns.
- Don't chain siblings ("first `/worktree-research`, then `/fan-out`"). One delegation per call.
- Don't re-route on sibling failure. If the Skill tool surfaces an error, report it to the user — don't pick a second sibling and try again.
- Don't add `/worktree-merge` or `/complex-rebase` to the routing table even as a "just in case." Both are terminal siblings the user invokes directly.
- Don't hallucinate sibling names. The routing table is the canonical registry; if your candidate isn't in it, the right answer is *Novel workflow* or *Out-of-family*.

## Adding a new sibling

When a new `/worktree-<x>` skill is authored, add one row to the routing table above with its trigger signal. Keep it tight — one row per sibling, ordered most-specific first.
